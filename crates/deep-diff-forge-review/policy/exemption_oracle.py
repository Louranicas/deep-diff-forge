#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`, with everything inside such an item.
Councils r12-r16 showed that judging what an exclusion hides by observing builds can always be
outflanked by a configuration the oracle did not sample (features, profiles, targets). So the
oracle no longer classifies exclusions; it allow-lists them:

1. List the scope three times, all with the same pinned cargo-mutants crate: the gate's build;
   `ddf-cargo-mutants-strict`, which honours only `#[cfg(test)]` (cargo-mutants' own
   `attr_is_cfg_test`: the whole argument must be the identifier `test`); and
   `ddf-cargo-mutants-noexclude`, which honours nothing. cargo-mutants' own `syn` parse decides
   all three, so there is no second parser to diverge from it.
2. A function the gate skips but the strict build lists was hidden by something other than
   `#[cfg(test)]`: `mutants::skip`, a test-suffixed attribute, `unsafe fn`, or a bare `#[test]`
   (an import can make `test` another macro, even under a feature: council r17). The run is
   refused outright, before any build, so no feature or profile combination can matter.
3. Code under `#[cfg(test)]` is test code by its source, unless a build script prints
   `cargo:rustc-cfg=test`. That is checked by changing the code and watching the compiler: the
   snapshot is copied once to a cargo-mutants-style directory and built as the gate builds it
   (`cargo build --profile test --lib --bins --tests`, measured to reproduce cargo test's units)
   and in release, with MIR emitted (`--emit=link,mir`, which the mutation job carries too).
   Neutralising the allow-listed bodies (`loop {}` and `panic!()`, keeping lines) must leave the
   canonical MIR of every non-test unit of every workspace member unchanged (council r17: a
   `#[path]` include can ship the code from another member), and every build must succeed.
4. Controls, in both configurations: the unmodified build succeeds and reproduces, and
   neutralising a function the gate mutates keeps every unit building and changes one. After
   every edited build, the edited files must still hold what the oracle wrote.

A build script or proc macro that detects this job and behaves differently, or that keys
`cfg(test)` to a configuration not built here, is the documented "lying to the tool" residual
(docs/REVIEW_COUNCIL.md). Exit status: 0 passed, 1 refused, 2 inconclusive.
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
STRICT = "ddf-cargo-mutants-strict"
SCHEMA = "deep-diff-forge.exemption-oracle.v7"
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
LIBRARY_EXTENSIONS = {"rlib", "rmeta", "so", "a", "dylib", "dll"}
REPLACEMENTS = ("loop {}", "panic!()")
EMIT = "--emit=link,mir"
# The gate's own units, and release: the configuration a build script most often keys on.
CONFIGS = {
    "gate": (["--profile", "test", "--lib", "--bins", "--tests"], "debug"),
    "release": (["--release", "--lib", "--bins"], "release"),
}
CONTROL_TRIES = 5
# Allow-listed functions named individually when their batch ships (4 builds each).
ATTRIBUTION_LIMIT = 8
# Build trees this process created, removed on exit: each holds a full target directory.
TREES = []
# Colour-refinement rounds: each is a pure function of the last, so any bound stays
# order-independent; allocation chains deeper than this are merely less distinguished.
REFINEMENT_ROUNDS = 16


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
    """Drop body spans nested in another of the same file: the outer replacement removes them.
    One sort and one sweep (council r17: the pairwise check was quadratic)."""
    kept, current = [], None
    for path, span in sorted(spans, key=lambda item: (item[0], position(item[1], False),
                                                       tuple(-v for v in position(item[1], True)))):
        if current and current[0] == path and position(span, True) <= position(current[1], True):
            continue  # inside the span kept last
        kept.append((path, span))
        current = (path, span)
    return kept


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
# A string literal is kept whole (on one line), so `"alloc1"` in program data is never a reference;
# outside one, `allocN` is a reference only in MIR's reference syntax (`{alloc7: &str}`,
# `{alloc7<imm>: ..}`, `╾alloc7<imm>╼`, `alloc7+0x4`), never an identifier such as `fn alloc7`
# (council r17).
ALLOC_TOKEN = re.compile(r'"(?:[^"\\\n]|\\.)*"|(?<=[{╾─])alloc(\d+)(?=[<:+╼─])')


def substitute(text, label):
    """Rename allocation references; an `allocN` that names no allocation (an identifier such as
    `fn alloc7`) is left as written."""
    return ALLOC_TOKEN.sub(lambda m: m[0] if m[1] is None else label(m[1]) or m[0], text)


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
    labels = {n: "" for n in allocations}
    for _ in range(REFINEMENT_ROUNDS):
        refined = {n: hashlib.sha256(substitute(c, lambda r: "alloc<" + labels[r] + ">" if r in labels else None)
                                     .encode()).hexdigest()[:24] for n, c in allocations.items()}
        stable = len(set(refined.values())) == len(set(labels.values()))
        labels = refined
        if stable:
            break

    def name(r):
        return "alloc<" + labels[r] + ">" if r in labels else None

    # A sorted list, not a set: how many allocations share a label is part of the program.
    definitions = sorted(labels[n] + substitute(c, name) for n, c in allocations.items())
    return substitute("\n".join(body), name) + "\n" + "\n".join(definitions)


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
        TREES.append(self.tree)
        # A Cargo build directory carries CACHEDIR.TAG; a source directory named `target` does not.
        stale = os.path.exists(os.path.join(source, "target", "CACHEDIR.TAG"))
        shutil.copytree(source, self.tree, symlinks=True, dirs_exist_ok=True,
                        ignore=lambda d, names: {"target"} if stale and d == source else set())
        metadata = json.loads(run(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
                                  cwd=self.tree).stdout)
        members = [p for p in metadata["packages"] if p["name"] == package]
        if len(members) != 1:
            raise RuntimeError(f"package {package} is not exactly one workspace member")
        # Every member's non-test units are observed: a `#[path]` include can ship the mutation
        # package's code from another member (council r17).
        self.manifests = {p["manifest_path"] for p in metadata["packages"]}
        self.roots = sorted({t["src_path"] for t in members[0]["targets"]})
        self.expected = {}
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
        """Canonical MIR digests of the package's non-test units that built. A unit missing from an
    edited build reads as changed (fail closed); the unmodified build must succeed completely."""
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
            self.expected[path] = text
        for root in touch:
            os.utime(root)
        self.refuse_runtime_config()
        arguments, subdirectory = CONFIGS[config]
        deps = os.path.join(self.tree, "target", subdirectory, "deps")
        # --keep-going: a test harness that a neutralised test-only `const fn` breaks must not hide
        # whether the non-test units still build.
        done = run(["cargo", "build", "--workspace", *arguments, "--locked", "--keep-going",
                    "--message-format=json"], cwd=self.tree, env=self.env, check=False)
        self.refuse_runtime_config()
        # A build script runs in this tree with write access; had it restored a neutralised file,
        # the unchanged MIR would prove nothing (council r17).
        for path in self.edited:
            with open(os.path.join(self.tree, path), encoding="utf-8", errors="replace") as now:
                if now.read() != self.expected[path]:
                    raise RuntimeError(f"{path} changed during the build")
        if done.returncode != 0 and not spans:
            raise RuntimeError(f"the unmodified tree does not build in {config}: {done.stderr[-600:]}")
        # Cargo reports uplifted paths for some units; each is a hard link to its deps/ original.
        twins = {}
        for path in glob.glob(os.path.join(deps, "*")):
            if os.path.isfile(path) and not path.endswith((".d", ".mir")):
                twins.setdefault(os.stat(path).st_ino, []).append(path)
        digests = {}
        for line in done.stdout.splitlines():
            message = json.loads(line) if line.startswith("{") else {}
            if message.get("reason") != "compiler-artifact" or message.get("manifest_path") not in self.manifests:
                continue
            kinds = set(message["target"]["kind"])
            if message["profile"]["test"] or not kinds & (LIBRARY_KINDS | {"bin"}):
                continue  # test harnesses, build scripts, examples: not shipped code
            stems = set()
            for name in message["filenames"]:
                for twin in ([name] if os.path.dirname(name) == deps else twins.get(os.stat(name).st_ino, [])):
                    if os.path.exists(os.path.join(deps, f"{stem_of(twin)}.mir")):
                        stems.add(stem_of(twin))
            if not stems:
                raise RuntimeError(f"no MIR found for {message['target']['name']} ({message['filenames']})")
            for stem in stems:
                with open(os.path.join(deps, f"{stem}.mir"), encoding="utf-8", errors="replace") as mir:
                    digests[stem] = digest_text(mir.read())
        if not digests and not spans:
            raise RuntimeError(f"configuration {config} produced no non-test lib or bin")
        return digests


def judge(package, in_diff):
    scope = ["--package", package] + (["--in-diff", in_diff] if in_diff else [])
    stock, stock_bodies = listing("cargo", scope)
    strict, _ = listing(STRICT, scope)
    everything, bodies = listing(NOEXCLUDE, scope)
    hidden = everything - stock
    # Hidden by something other than exact `#[cfg(test)]` or a bare `#[test]`: refused outright.
    disallowed = sorted(hidden & strict)
    allowed = sorted(hidden - strict)
    verdict = {"schema": SCHEMA, "package": package, "in_diff": bool(in_diff), "hidden_functions": len(hidden),
               "allow_listed": len(allowed), "builds": 0, "refused": [], "reasons": [], "configurations": {}}
    verdict["refused"] = [{"file": f, "function": n, "line": line,
                           "reason": "excluded by something other than #[cfg(test)]"}
                          for f, n, line, _ in disallowed]
    if disallowed or not allowed:
        # The verdict is already certain: build nothing (council r17).
        verdict["state"] = "refused" if disallowed else "passed"
        return verdict
    unbodied = [key for key in allowed if key not in bodies]
    if unbodied:
        verdict["reasons"].append(f"{len(unbodied)} allow-listed functions have no body to neutralise: {unbodied[:5]}")
    build = GateBuild(os.getcwd(), package)
    original = {config: build.build(config) for config in CONFIGS}
    verdict["configurations"] = {config: "built" for config in original}
    # Both configurations get a reproducibility check and a positive control (council r17: a
    # release comparison that cannot move would otherwise pass unnoticed).
    verdict["reproducible"] = all(build.build(config, touch=build.roots) == original[config] for config in CONFIGS)
    if not verdict["reproducible"]:
        verdict["reasons"].append("two builds of the unmodified tree disagree: the comparison cannot be trusted")
    if not stock_bodies:
        stock, stock_bodies = listing("cargo", ["--package", package])
    verdict["control"] = {"mir_changed": False}
    for control in sorted(key for key in stock if key in stock_bodies)[:CONTROL_TRIES]:
        moved = []
        for config in CONFIGS:
            edited = build.build(config, [(control[0], stock_bodies[control])], REPLACEMENTS[0])
            moved.append(set(edited) == set(original[config]) and edited != original[config])
        if all(moved):
            verdict["control"] = {"file": control[0], "function": control[1], "mir_changed": True,
                                  "configurations": sorted(CONFIGS)}
            break
    if not verdict["control"]["mir_changed"]:
        verdict["reasons"].append("no mutated function built completely and changed non-test MIR in every configuration")
    spans = [(key[0], bodies[key]) for key in allowed if key in bodies]
    def shipping(selected):
        return sorted({config for config in CONFIGS for replacement in REPLACEMENTS
                       if build.build(config, selected, replacement) != original[config]})

    shipped_in = shipping(spans)
    verdict["shipping_configurations"] = shipped_in
    if shipped_in:
        # Allow-listed by source yet compiled into shipped code: a shadowed `#[test]` or a build
        # script setting cfg(test). Named one by one when few; otherwise the batch is refused whole.
        named = [key for key in allowed if key in bodies]
        if len(named) <= ATTRIBUTION_LIMIT:
            named = [key for key in named if shipping([(key[0], bodies[key])])]
        verdict["refused"] += [{"file": f, "function": n, "line": line,
                                "reason": "allow-listed but compiled into " + ", ".join(shipped_in)}
                               for f, n, line, _ in named or allowed]
    verdict["builds"] = build.count
    verdict["state"] = "inconclusive" if verdict["reasons"] else ("refused" if verdict["refused"] else "passed")
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
    finally:
        for tree in TREES:
            shutil.rmtree(tree, ignore_errors=True)
    with open(args.witness, "w", encoding="utf-8") as out:
        json.dump(verdict, out, indent=1)
    print(json.dumps({k: v for k, v in verdict.items() if k != "refused"} | {"refused": len(verdict["refused"])}))
    for row in verdict["refused"][:50]:
        print(f"refused ({row['reason']}): {row['file']}:{row['line']} {row['function']}", file=sys.stderr)
    return {"passed": 0, "refused": 1}.get(verdict["state"], 2)


if __name__ == "__main__":
    sys.exit(main())
