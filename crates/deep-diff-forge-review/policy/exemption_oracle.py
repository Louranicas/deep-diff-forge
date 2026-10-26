#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`, together with everything inside such an
item. A candidate reaches those exclusions in spellings no syntactic scan predicts (councils r12-r15).

An exclusion is legitimate only for test code: code that is compiled into a test harness of the
gate's build and into no non-test unit of any configuration that ships. Code compiled nowhere is
not test code either: listed, its mutants would survive and hold the gate (council r14). This
oracle checks both halves by changing the hidden code and watching the compiler:

1. List the scope with the gate's cargo-mutants and with `ddf-cargo-mutants-noexclude` (the same
   pinned crate with attribute and `unsafe fn` exclusions off). Hidden = listed only by the second.
   With nothing hidden, the oracle passes unbuilt.
2. Copy the snapshot once to a cargo-mutants-style directory (`$TMPDIR/cargo-mutants-src-*.tmp`,
   target inside). Build, with `--keep-going`, the gate configuration (`--profile test --lib --bins
   --tests`: the gate's own `cargo test --no-run` units, measured equal even with a customised
   `[profile.test]`) and the shipping configurations a test build never compiles (`--release`,
   `--all-features`, `--no-default-features`, `--release --all-features`; council r15). Every
   unmodified build must succeed completely, and the gate configuration must reproduce.
3. Observable: per-unit MIR (`--emit=link,mir`, which the controller also gives the mutation job,
   so it is no tell), canonicalised by an order-independent refinement of allocation contents.
4. Neutralise hidden bodies (cargo-mutants' `FnValue` spans) with `loop {}` and with `panic!()`,
   keeping lines. A function ships if, in any configuration, a replacement changes a non-test
   unit's MIR or breaks a non-test unit with an error in an edited file. It is test code only if,
   in the gate configuration, its own MIR section in a harness changes, or a harness breaks with an
   error inside its body. Anything else was compiled nowhere and is refused.
5. Positive control: neutralising a function the gate mutates must leave every non-test unit
   building and change at least one.

Candidate build scripts and proc macros run during the builds; one that detects this job (the
parent process, timing) and builds differently, or forges compiler output, is the documented "lying
to the tool" residual (docs/REVIEW_COUNCIL.md). Other target triples are not built here. Exit
status: 0 passed, 1 refused, 2 inconclusive.
"""
import argparse
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

NOEXCLUDE = "ddf-cargo-mutants-noexclude"
SCHEMA = "deep-diff-forge.exemption-oracle.v5"
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
LIBRARY_EXTENSIONS = {"rlib", "rmeta", "so", "a", "dylib", "dll"}
REPLACEMENTS = ("loop {}", "panic!()")
EMIT = "--emit=link,mir"
GATE = "gate"
# The gate configuration builds test harnesses; the others are shipping configurations it never
# compiles. Each is (cargo arguments, target subdirectory).
CONFIGS = {
    GATE: (["--profile", "test", "--lib", "--bins", "--tests"], "debug"),
    "release": (["--release", "--lib", "--bins"], "release"),
    "all-features": (["--profile", "test", "--all-features", "--lib", "--bins"], "debug"),
    "no-default-features": (["--profile", "test", "--no-default-features", "--lib", "--bins"], "debug"),
    "release-all-features": (["--release", "--all-features", "--lib", "--bins"], "release"),
}
# Individual proofs (one build pair each) when a function's MIR section is not found by name.
INDIVIDUAL_LIMIT = 64
CONTROL_TRIES = 5


def run(argv, cwd=None, env=None, check=True):
    """Run a tool with both streams captured by communicate(), so a full pipe cannot deadlock."""
    done = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, check=False)
    if check and done.returncode != 0:
        raise RuntimeError(f"{' '.join(argv[:4])} exited {done.returncode}: {done.stderr[-600:]}")
    return done


def listing(tool, scope):
    """Functions a cargo-mutants listing names, keyed by file, name and start, with body spans."""
    argv = ["cargo", "mutants"] if tool == "cargo" else [tool, "mutants"]
    functions, bodies = set(), {}
    for mutant in json.loads(run(argv + ["--list", "--json", "--no-config", *scope]).stdout or "[]"):
        function = mutant.get("function")
        if not isinstance(function, dict):
            continue
        start = function["span"]["start"]
        key = (mutant["file"], function["function_name"], start["line"], start["column"])
        functions.add(key)
        if mutant.get("genre") == "FnValue":
            bodies[key] = mutant["span"]
    return functions, bodies


def position(span, end):
    point = span["end" if end else "start"]
    return point["line"], point["column"]


def outermost(spans):
    """Drop body spans nested in another of the same file: the outer replacement removes them."""
    return [(path, span) for path, span in spans
            if not any(other_path == path and other is not span
                       and position(other, False) <= position(span, False)
                       and position(span, True) <= position(other, True)
                       for other_path, other in spans)]


def neutralised(text, spans, replacement):
    """`text` with each body span (1-based lines, 1-based end-exclusive character columns) replaced,
    keeping every line's number and, where the body's first line is long enough, every column."""
    lines = text.split("\n")
    for span in sorted(spans, key=lambda s: position(s, False), reverse=True):
        (l1, c1), (l2, c2) = [(line - 1, column - 1) for line, column in (position(span, False), position(span, True))]
        if l1 == l2:
            body = lines[l1][c1:c2]
        else:
            body = "\n".join([lines[l1][c1:], *lines[l1 + 1:l2], lines[l2][:c2]])
        blank = re.sub(r"[^\n]", " ", body)
        fits = len(blank.split("\n", 1)[0]) >= len(replacement)
        new = replacement + (blank[len(replacement):] if fits else blank)
        lines[l1:l2 + 1] = (lines[l1][:c1] + new + lines[l2][c2:]).split("\n")
    return "\n".join(lines)


ALLOC_HEAD = re.compile(r"^alloc(\d+) (\(.*\)) \{(\})?$")
# A string literal is kept whole, so `"alloc1"` in program data is never taken for a reference.
ALLOC_TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|\balloc(\d+)\b')


def substitute(text, names):
    return ALLOC_TOKEN.sub(lambda m: m[0] if m[1] is None else names(m[1]), text)


def canonical_mir(text):
    """MIR with every allocation named by an order-independent refinement of its contents: rustc
    numbers and orders allocations by interning, which incremental reuse changes (measured), and
    allocations may reference each other in cycles (council r15)."""
    lines, body, allocations, i = text.split("\n"), [], {}, 0
    while i < len(lines):
        head = ALLOC_HEAD.match(lines[i])
        if head and head[3]:
            allocations[head[1]] = head[2] + " {}"
        elif head:
            end = i + 1
            while end < len(lines) and lines[end] != "}":
                end += 1
            allocations[head[1]] = head[2] + "\n" + "\n".join(lines[i + 1:end])
            i = end
        else:
            body.append(lines[i])
        i += 1
    referenced = {m[1] for m in ALLOC_TOKEN.finditer("\n".join(body) + "\n".join(allocations.values())) if m[1]}
    missing = referenced - set(allocations)
    if missing:
        raise RuntimeError(f"MIR references undefined allocations {sorted(missing)[:5]}")
    # Colour refinement: start from contents with references blanked, then repeatedly fold in the
    # neighbours' labels until the partition stops splitting. The result does not depend on
    # allocation numbers or visit order, and handles cycles.
    labels = {n: "" for n in allocations}
    for _ in range(len(allocations) + 1):
        refined = {n: hashlib.sha256(substitute(c, lambda r: "alloc<" + labels[r] + ">").encode()).hexdigest()[:24]
                   for n, c in allocations.items()}
        stable = len(set(refined.values())) == len(set(labels.values()))
        labels = refined
        if stable:
            break
    named = substitute("\n".join(body), lambda r: "alloc<" + labels[r] + ">")
    definitions = sorted({labels[n] + substitute(c, lambda r: "alloc<" + labels[r] + ">") for n, c in allocations.items()})
    return named + "\n" + "\n".join(definitions)


SECTION = re.compile(r"^(?:const )?fn (.+?)\(", re.M)


def sections(text):
    """{function path: canonical body} for each `fn` in a MIR dump."""
    canonical = canonical_mir(text)
    starts = [(m.start(), m[1]) for m in SECTION.finditer(canonical)]
    return {name: canonical[start:(starts[k + 1][0] if k + 1 < len(starts) else len(canonical))]
            for k, (start, name) in enumerate(starts)}


IMPL_PATH = re.compile(r"<impl at (\S+?):(\d+):\d+: (\d+):\d+>::([^:<>]+)$")


def section_of(key, names):
    """The one MIR section that is this hidden function, or None when no unique match exists (the
    caller then proves it with an individual build). A free function matches by its module-path
    suffix; a method by the `<impl at file:line>` span that contains its own start line."""
    path, function, line = key[0], key[1], key[2]
    leaf = function.split("::")[-1]
    matches = [n for n in names if n == function or n.endswith("::" + function)]
    for name in names:
        impl = IMPL_PATH.search(name)
        if impl and impl[4] == leaf and (path.endswith(impl[1]) or impl[1].endswith(path)) \
                and int(impl[2]) <= line <= int(impl[3]):
            matches.append(name)
    return matches[0] if len(set(matches)) == 1 else None


def digest_text(text):
    return hashlib.sha256(canonical_mir(text).encode()).hexdigest()


def stem_of(path):
    """The MIR stem of an artifact: `lib` is a prefix of library files only (council r15)."""
    name, _, extension = os.path.basename(path).partition(".")
    return name.removeprefix("lib") if extension in LIBRARY_EXTENSIONS else name


class GateBuild:
    """The snapshot copied once to a cargo-mutants-style path and built as the gate builds it."""

    def __init__(self, source, package):
        self.source, self.package, self.count, self.edited = source, package, 0, set()
        name = os.path.basename(source.rstrip("/")) or "src"
        self.tree = tempfile.mkdtemp(prefix=f"cargo-mutants-{name}-", suffix=".tmp")
        # A Cargo build directory carries CACHEDIR.TAG; a source directory named `target` does not.
        stale = os.path.exists(os.path.join(source, "target", "CACHEDIR.TAG"))
        shutil.copytree(source, self.tree, symlinks=True, dirs_exist_ok=True,
                        ignore=lambda d, names: {"target"} if stale and d == source else set())
        metadata = json.loads(run(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
                                  cwd=self.tree).stdout)
        members = [p for p in metadata["packages"] if p["name"] == package]
        if len(members) != 1:
            raise RuntimeError(f"package {package} is not exactly one workspace member")
        self.manifest = members[0]["manifest_path"]
        self.roots = sorted({t["src_path"] for t in members[0]["targets"]})
        # The mutation gate's environment: cargo-mutants adds these two, and the controller gives
        # both jobs the emitted-MIR flag.
        self.env = dict(os.environ, INSTA_UPDATE="no", INSTA_FORCE_PASS="0")
        self.env.setdefault("CARGO_ENCODED_RUSTFLAGS", EMIT)
        self.env.pop("CARGO_TARGET_DIR", None)
        if EMIT not in self.env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"):
            raise RuntimeError("the build environment does not emit MIR")
        self.home = os.path.realpath(self.env.get("CARGO_HOME", os.path.expanduser("~/.cargo")))

    def refuse_runtime_config(self):
        """Cargo reads `.cargo/config*` from the build directory and every ancestor. Admission
        refuses candidate config in the snapshot; a build script could still write one at runtime
        to steer a later build (council r15), so none may exist outside CARGO_HOME."""
        directory = os.path.realpath(self.tree)
        while True:
            for name in ("config", "config.toml"):
                found = os.path.join(directory, ".cargo", name)
                if os.path.exists(found) and os.path.realpath(os.path.dirname(found)) != self.home:
                    raise RuntimeError(f"runtime Cargo configuration appeared at {found}")
            parent = os.path.dirname(directory)
            if parent == directory:
                return
            directory = parent

    def build(self, config, spans=(), replacement=None, touch=()):
        """{"nontest": {stem: digest}, "harness": {stem: text}, "errors": [(file, line)], "ok": bool}."""
        for path in self.edited:
            shutil.copyfile(os.path.join(self.source, path), os.path.join(self.tree, path))
        self.edited = set()
        self.count += 1
        by_file = {}
        for path, span in outermost(list(spans)):
            by_file.setdefault(path, []).append(span)
        for path, file_spans in by_file.items():
            with open(os.path.join(self.source, path), encoding="utf-8") as original:
                text = neutralised(original.read(), file_spans, replacement)
            with open(os.path.join(self.tree, path), "w", encoding="utf-8") as out:
                out.write(text)
            self.edited.add(path)
        for root in touch:
            os.utime(root)
        self.refuse_runtime_config()
        arguments, subdirectory = CONFIGS[config]
        deps = os.path.join(self.tree, "target", subdirectory, "deps")
        done = run(["cargo", "build", "--workspace", *arguments, "--locked", "--keep-going",
                    "--message-format=json"], cwd=self.tree, env=self.env, check=False)
        self.refuse_runtime_config()
        # Cargo reports uplifted paths for some units; each is a hard link to its deps/ original.
        twins = {}
        for path in glob.glob(os.path.join(deps, "*")):
            if os.path.isfile(path) and not path.endswith((".d", ".mir")):
                twins.setdefault(os.stat(path).st_ino, []).append(path)
        result = {"nontest": {}, "harness": {}, "errors": [], "ok": done.returncode == 0}
        for line in done.stdout.splitlines():
            message = json.loads(line) if line.startswith("{") else {}
            if message.get("reason") == "compiler-message" and message["message"].get("level") == "error":
                result["errors"] += [(s["file_name"], s["line_start"], s["line_end"])
                                     for s in message["message"].get("spans", [])]
            if message.get("reason") != "compiler-artifact" or message.get("manifest_path") != self.manifest:
                continue
            kinds, test = set(message["target"]["kind"]), message["profile"]["test"]
            if not (test or kinds & (LIBRARY_KINDS | {"bin"})):
                continue  # build scripts, examples: not the package's shipped or test code
            stems = set()
            for name in message["filenames"]:
                for twin in ([name] if os.path.dirname(name) == deps else twins.get(os.stat(name).st_ino, [])):
                    if os.path.exists(os.path.join(deps, f"{stem_of(twin)}.mir")):
                        stems.add(stem_of(twin))
            if not stems:
                raise RuntimeError(f"no MIR found for {message['target']['name']} ({message['filenames']})")
            for stem in stems:
                with open(os.path.join(deps, f"{stem}.mir"), encoding="utf-8", errors="replace") as mir:
                    text = mir.read()
                if test:
                    result["harness"][stem] = text
                else:
                    result["nontest"][stem] = digest_text(text)
        if not spans and not result["ok"]:
            raise RuntimeError(f"the unmodified tree does not build in configuration {config}: {done.stderr[-600:]}")
        if not spans and not result["nontest"]:
            raise RuntimeError(f"configuration {config} produced no non-test lib or bin of {self.package}")
        return result


def in_body(errors, key, body):
    """Whether an error points inside this function's body (a harness broken by its edit)."""
    start, end = body["start"]["line"], body["end"]["line"]
    return any(file == key[0] and line_start <= end and line_end >= start for file, line_start, line_end in errors)


def judge(package, in_diff):
    scope = ["--package", package] + (["--in-diff", in_diff] if in_diff else [])
    stock, stock_bodies = listing("cargo", scope)
    examined, bodies = listing(NOEXCLUDE, scope)
    hidden = sorted(examined - stock)
    verdict = {"schema": SCHEMA, "package": package, "in_diff": bool(in_diff),
               "hidden_functions": len(hidden), "builds": 0, "refused": [], "reasons": [], "configurations": {}}
    if not hidden:
        verdict["state"] = "passed"
        return verdict
    unbodied = [key for key in hidden if key not in bodies]
    if unbodied:
        verdict["reasons"].append(f"{len(unbodied)} hidden functions have no body to neutralise: {unbodied[:5]}")
    build = GateBuild(os.getcwd(), package)
    original = {}
    for config in CONFIGS:
        try:
            original[config] = build.build(config)
            verdict["configurations"][config] = "built"
        except RuntimeError as error:
            if config == GATE:
                raise
            # A configuration that does not build cannot ship anything.
            verdict["configurations"][config] = f"does not build: {str(error)[-200:]}"
    gate = original[GATE]
    verdict["reproducible"] = build.build(GATE, touch=build.roots)["nontest"] == gate["nontest"]
    if not verdict["reproducible"]:
        verdict["reasons"].append("two builds of the unmodified tree disagree: the comparison cannot be trusted")
    edited_files = {key[0] for key in hidden}

    def ships(config, result):
        """A non-test unit changed, or one broke with an error in an edited file."""
        base = original[config]["nontest"]
        if any(result["nontest"].get(stem, digest) != digest for stem, digest in base.items() if stem in result["nontest"]):
            return True
        broken = set(base) - set(result["nontest"])
        return bool(broken) and any(file in edited_files for file, _, _ in result["errors"])

    if not stock_bodies:
        stock, stock_bodies = listing("cargo", ["--package", package])
    verdict["control"] = {"mir_changed": False}
    for control in sorted(key for key in stock if key in stock_bodies)[:CONTROL_TRIES]:
        edited = build.build(GATE, [(control[0], stock_bodies[control])], REPLACEMENTS[0])
        complete = set(edited["nontest"]) == set(gate["nontest"])
        if complete and edited["nontest"] != gate["nontest"]:
            verdict["control"] = {"file": control[0], "function": control[1], "mir_changed": True}
            break
    if not verdict["control"]["mir_changed"]:
        verdict["reasons"].append("no mutated function both built completely and changed non-test MIR when neutralised")

    neutralisable = [key for key in hidden if key in bodies]
    spans = [(key[0], bodies[key]) for key in neutralisable]
    shipped_in = set()
    proven = set()
    gate_sections = {stem: sections(text) for stem, text in gate["harness"].items()}
    for replacement in REPLACEMENTS:
        for config in original:
            result = build.build(config, spans, replacement)
            if ships(config, result):
                shipped_in.add(config)
            if config == GATE:
                after = {stem: sections(text) for stem, text in result["harness"].items()}
                for key in neutralisable:
                    for stem, base in gate_sections.items():
                        name = section_of(key, base)
                        if name is not None and stem in after and after[stem].get(name) != base[name]:
                            proven.add(key)
                    if set(gate["harness"]) - set(result["harness"]) and in_body(result["errors"], key, bodies[key]):
                        proven.add(key)
    verdict["shipping_configurations"] = sorted(shipped_in)

    refused, individual = [], 0
    for key in neutralisable:
        body = bodies[key]
        one_ships = False
        if shipped_in:
            # Name what ships: only the configurations where the batch showed it.
            for config in sorted(shipped_in):
                for replacement in REPLACEMENTS:
                    individual += 1
                    if individual > INDIVIDUAL_LIMIT:
                        break
                    if ships(config, build.build(config, [(key[0], body)], replacement)):
                        one_ships = True
                        break
                if one_ships:
                    break
        if not one_ships and key not in proven:
            for replacement in REPLACEMENTS:
                individual += 1
                if individual > INDIVIDUAL_LIMIT:
                    break
                result = build.build(GATE, [(key[0], body)], replacement)
                moved = any(result["harness"].get(stem) is not None and digest_text(result["harness"][stem]) != digest_text(text)
                            for stem, text in gate["harness"].items())
                broke = set(gate["harness"]) - set(result["harness"]) and in_body(result["errors"], key, body)
                if moved or broke:
                    proven.add(key)
                    break
        if individual > INDIVIDUAL_LIMIT:
            verdict["reasons"].append(f"more than {INDIVIDUAL_LIMIT} individual builds needed to prove hidden functions")
            break
        if one_ships or key not in proven:
            refused.append({"file": key[0], "function": key[1], "line": key[2],
                            "reason": "ships" if one_ships else "compiled nowhere"})
    if shipped_in and not any(row["reason"] == "ships" for row in refused):
        refused.append({"file": None, "function": f"{len(neutralisable)} hidden functions together", "line": None,
                        "reason": "ships in " + ", ".join(sorted(shipped_in))})
    verdict["refused"] = refused
    verdict["builds"] = build.count
    verdict["state"] = "inconclusive" if verdict["reasons"] else ("refused" if refused else "passed")
    return verdict


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", required=True)
    parser.add_argument("--witness", required=True)
    parser.add_argument("--in-diff")
    args = parser.parse_args()
    try:
        verdict = judge(args.package, args.in_diff)
    except (RuntimeError, OSError, ValueError, KeyError, StopIteration, IndexError) as error:
        verdict = {"schema": SCHEMA, "state": "inconclusive", "package": args.package,
                   "hidden_functions": None, "builds": 0, "refused": [], "reasons": [str(error)[-1000:]]}
    with open(args.witness, "w", encoding="utf-8") as out:
        json.dump(verdict, out, indent=1)
    print(json.dumps({k: v for k, v in verdict.items() if k != "refused"} | {"refused": len(verdict["refused"])}))
    for row in verdict["refused"][:50]:
        print(f"refused ({row['reason']}): {row['file']}:{row['line']} {row['function']}", file=sys.stderr)
    return {"passed": 0, "refused": 1}.get(verdict["state"], 2)


if __name__ == "__main__":
    sys.exit(main())
