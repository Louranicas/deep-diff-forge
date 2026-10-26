#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`, together with everything inside such an
item. A candidate reaches those exclusions in spellings no syntactic scan predicts: an imported macro
shadowing the built-in `#[test]`, a build script printing `cargo:rustc-cfg=test`, or an attribute
macro that renames or re-spans the item or an enclosing module (councils r12 and r13).

An exclusion is legitimate only for test code. Code the gate never compiles is not test code
either: listed, its mutants would survive and hold the gate, so hiding it must not clear it
(council r14). This oracle therefore requires every hidden function to be genuine test code in the
gate's own build, by changing it and watching what that build compiles:

1. List the scope twice: with the gate's cargo-mutants and with `ddf-cargo-mutants-noexclude`, the
   same pinned crate built with attribute and `unsafe fn` exclusions switched off. A function only
   the second lists was hidden. With nothing hidden, the oracle passes unbuilt.
2. Copy the snapshot once to a cargo-mutants-style directory (`$TMPDIR/cargo-mutants-src-*.tmp`,
   target inside) and build the units the mutation gate's `cargo test --workspace --no-run` builds
   (`cargo build --workspace --lib --bins --tests --keep-going`: the same unit hashes, measured),
   which also builds the non-test binaries a test build links only for integration tests. The environment is the mutation gate's own: the controller gives both jobs
   `--emit=link,mir`, so the flag tells a build script nothing (council r14).
3. Observables: the package's non-test lib and bin MIR, and its test-harness MIR, with allocations
   renamed by content so incremental reordering cancels out (measured). The unmodified tree is built
   twice, the second time touching every target root, and the two must agree.
4. Neutralise hidden bodies (cargo-mutants' `FnValue` spans) with `loop {}` and with `panic!()`,
   keeping lines. A function ships if either replacement changes non-test MIR or stops a non-test
   unit building; it is genuine test code if it ships nowhere and some replacement changes or breaks
   a test harness. Anything else was compiled nowhere and is refused too.
5. Positive control: neutralising a function the gate mutates must build and change non-test MIR.

The controller embeds this file and runs it in the gate sandbox. Candidate build scripts and proc
macros run during the builds; one that detects this job anyway (the parent process, timing) and
builds differently, or forges compiler output, is the documented "lying to the tool" residual
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
SCHEMA = "deep-diff-forge.exemption-oracle.v4"
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
REPLACEMENTS = ("loop {}", "panic!()")
EMIT = "--emit=link,mir"
# Functions individually re-built to name what ships or to prove test code; beyond, unnamed.
ATTRIBUTION_LIMIT = 64
CONTROL_TRIES = 5
FAILED = "build failed"


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
    kept = []
    for path, span in spans:
        inside = any(other_path == path and other is not span
                     and position(other, False) <= position(span, False)
                     and position(span, True) <= position(other, True)
                     for other_path, other in spans)
        if not inside:
            kept.append((path, span))
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
ALLOC_REF = re.compile(r"\balloc(\d+)\b")


def canonical_mir(text):
    """MIR with each allocation named by a hash of its contents and the definitions sorted: rustc
    numbers and orders allocations by interning, which incremental reuse changes (measured)."""
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
    names = {}

    def name(number, seen=()):
        if number in names:
            return names[number]
        if number not in allocations or number in seen:
            return f"alloc<unresolved {number in seen}>"
        content = ALLOC_REF.sub(lambda m: name(m[1], (*seen, number)), allocations[number])
        names[number] = "alloc<" + hashlib.sha256(content.encode()).hexdigest()[:24] + ">"
        return names[number]

    definitions = sorted({name(n) + ALLOC_REF.sub(lambda m: name(m[1]), c) for n, c in allocations.items()})
    return ALLOC_REF.sub(lambda m: name(m[1]), "\n".join(body)) + "\n" + "\n".join(definitions)


def digest_file(path):
    with open(path, encoding="utf-8", errors="replace") as mir:
        return hashlib.sha256(canonical_mir(mir.read()).encode()).hexdigest()


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
        self.deps = os.path.join(self.tree, "target", "debug", "deps")
        # The mutation gate's environment: cargo-mutants adds these two, and the controller gives
        # both jobs the emitted-MIR flag.
        self.env = dict(os.environ, INSTA_UPDATE="no", INSTA_FORCE_PASS="0")
        self.env.setdefault("CARGO_ENCODED_RUSTFLAGS", EMIT)
        self.env.pop("CARGO_TARGET_DIR", None)
        if EMIT not in self.env["CARGO_ENCODED_RUSTFLAGS"].split("\x1f"):
            raise RuntimeError("the build environment does not emit MIR")

    def artifacts(self, argv):
        """(test, kinds, deps stems, executable) for each artifact of the package a build made."""
        done = run(argv, cwd=self.tree, env=self.env, check=False)
        self.stderr = done.stderr[-600:]
        # Cargo reports uplifted paths (target/debug/libfoo.rlib, target/debug/foo) for some units;
        # each is a hard link to its deps/ original, whose name carries the unit's MIR stem.
        twins = {}
        for path in glob.glob(os.path.join(self.deps, "*")):
            if os.path.isfile(path) and not path.endswith((".d", ".mir")):
                twins.setdefault(os.stat(path).st_ino, []).append(path)
        found = []
        for line in done.stdout.splitlines():
            message = json.loads(line) if line.startswith("{") else {}
            if message.get("reason") != "compiler-artifact" or message.get("manifest_path") != self.manifest:
                continue
            kinds, test = set(message["target"]["kind"]), message["profile"]["test"]
            if not (test or kinds & (LIBRARY_KINDS | {"bin"})):
                continue  # build scripts, examples: not the package's shipped or test code
            stems = set()
            for name in message["filenames"]:
                for twin in ([name] if os.path.dirname(name) == self.deps
                             else twins.get(os.stat(name).st_ino, [])):
                    stem = os.path.basename(twin).split(".")[0].removeprefix("lib")
                    if os.path.exists(os.path.join(self.deps, f"{stem}.mir")):
                        stems.add(stem)
            if not stems:
                raise RuntimeError(f"no MIR found for {message['target']['name']} ({message['filenames']})")
            found.append((test, kinds, stems))
        return found

    def digest(self, spans=(), replacement=None, touch=()):
        """{"nontest": {...}, "harness": {...}}: canonical MIR digests per unit, FAILED for a unit
        that did not build. The unmodified tree must build completely."""
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
        units = {"nontest": {}, "harness": {}}
        # The units of the gate's `cargo test --workspace --no-run` (measured: the same unit hashes,
        # reused fresh), plus the non-test bins, which a test build links only for integration
        # tests; `cargo test` has no `--keep-going`, so one unit's failure cannot hide the others.
        built = self.artifacts(["cargo", "build", "--workspace", "--lib", "--bins", "--tests", "--locked",
                                "--keep-going", "--message-format=json"])
        for test, _, stems in built:
            for stem in stems:
                units["harness" if test else "nontest"][stem] = digest_file(os.path.join(self.deps, f"{stem}.mir"))
        if not units["nontest"]:
            if not spans:
                raise RuntimeError(f"the gate build produced no non-test lib or bin of {self.package}: {self.stderr}")
        return units


def compare(original, edited):
    """(non-test changed or broken, harness changed or broken) against the unmodified build."""
    def moved(kind):
        return any(edited[kind].get(unit, FAILED) != value for unit, value in original[kind].items())
    return moved("nontest"), moved("harness")


def judge(package, in_diff):
    scope = ["--package", package] + (["--in-diff", in_diff] if in_diff else [])
    stock, stock_bodies = listing("cargo", scope)
    examined, bodies = listing(NOEXCLUDE, scope)
    hidden = sorted(examined - stock)
    verdict = {"schema": SCHEMA, "package": package, "in_diff": bool(in_diff),
               "hidden_functions": len(hidden), "builds": 0, "refused": [], "reasons": []}
    if not hidden:
        verdict["state"] = "passed"
        return verdict
    unbodied = [key for key in hidden if key not in bodies]
    if unbodied:
        verdict["reasons"].append(f"{len(unbodied)} hidden functions have no body to neutralise: {unbodied[:5]}")
    build = GateBuild(os.getcwd(), package)
    original = build.digest()
    verdict["reproducible"] = build.digest(touch=build.roots) == original
    if not verdict["reproducible"]:
        verdict["reasons"].append("two builds of the unmodified tree disagree: the comparison cannot be trusted")
    if not stock_bodies:
        stock, stock_bodies = listing("cargo", ["--package", package])
    verdict["control"] = {"mir_changed": False}
    for control in sorted(key for key in stock if key in stock_bodies)[:CONTROL_TRIES]:
        edited = build.digest([(control[0], stock_bodies[control])], REPLACEMENTS[0])
        built = all(value != FAILED for value in edited["nontest"].values()) and edited["nontest"]
        if built and compare(original, edited)[0]:
            verdict["control"] = {"file": control[0], "function": control[1], "mir_changed": True}
            break
    if not verdict["control"]["mir_changed"]:
        verdict["reasons"].append("no mutated function both built and changed non-test MIR when neutralised")

    def effects(keys):
        """(ships, test code) for neutralising `keys` together, over both replacements."""
        ships = harness = False
        for replacement in REPLACEMENTS:
            moved_nontest, moved_harness = compare(original, build.digest(
                [(key[0], bodies[key]) for key in keys], replacement))
            ships, harness = ships or moved_nontest, harness or moved_harness
            if ships:
                break
        return ships, harness

    neutralisable = [key for key in hidden if key in bodies]
    ships, harness = effects(neutralisable)
    refused = []
    if ships or not harness or len(neutralisable) > 1:
        # Name each function: some may ship, and some may be compiled nowhere at all.
        if len(neutralisable) > ATTRIBUTION_LIMIT:
            if ships or not harness:
                refused.append({"file": None, "function": f"{len(neutralisable)} hidden functions",
                                "line": None, "reason": "ships" if ships else "compiled nowhere"})
            else:
                verdict["reasons"].append(f"more than {ATTRIBUTION_LIMIT} hidden functions to prove test code")
        else:
            for key in neutralisable:
                one_ships, one_harness = effects([key])
                if one_ships or not one_harness:
                    refused.append({"file": key[0], "function": key[1], "line": key[2],
                                    "reason": "ships" if one_ships else "compiled nowhere"})
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
