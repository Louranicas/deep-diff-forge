#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`, together with everything inside such an
item. A candidate reaches those exclusions in spellings no syntactic scan predicts: an imported macro
shadowing the built-in `#[test]`, a build script printing `cargo:rustc-cfg=test`, or an attribute
macro that renames or re-spans the item or an enclosing module (councils r12 and r13).

This oracle changes the hidden code and watches what the gate's own build compiles:

1. List the scope twice: with the gate's cargo-mutants and with `ddf-cargo-mutants-noexclude`, the
   same pinned crate built with attribute and `unsafe fn` exclusions switched off. A function only
   the second lists was hidden by an exclusion. With nothing hidden, the oracle passes unbuilt.
2. Copy the snapshot once to a cargo-mutants-style directory (`$TMPDIR/cargo-mutants-src-*.tmp`,
   target inside) and build it as the mutation gate does, `cargo test --workspace --no-run`, with
   MIR also emitted. Every build uses that one path, so paths, features, profile, dependency feature
   unification and target are the gate's own (council r13).
3. The observable is the package's non-test MIR (lib and bins, never test harnesses), with
   allocation numbers canonicalised. The unmodified tree is built twice, the second time forcing a
   recompile, and the two must agree.
4. Neutralise the hidden bodies (cargo-mutants' `FnValue` spans), once with `loop {}` and once with
   `panic!()`, keeping lines. Hidden code ships if either changes the MIR or breaks the build: a body
   that already equals one replacement cannot equal both (council r13), and code absent from the
   build cannot break it.
5. Positive control: neutralising a function the gate mutates must build and change the MIR.

The controller embeds this file and runs it in the gate sandbox. Candidate build scripts and proc
macros run during the builds and can still tell them apart from the gate's (the emitted-MIR flag,
the parent process); a candidate that does so and builds differently, or forges compiler output, is
the documented "lying to the tool" residual (docs/REVIEW_COUNCIL.md). Exit status: 0 passed,
1 refused, 2 inconclusive; the witness says which.
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
SCHEMA = "deep-diff-forge.exemption-oracle.v3"
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
REPLACEMENTS = ("loop {}", "panic!()")
# Functions individually re-built (with both replacements) to name what ships; beyond, unnamed.
ATTRIBUTION_LIMIT = 32
# Mutated functions tried as the positive control, since some (a `const fn`) cannot be neutralised.
CONTROL_TRIES = 5


def run(argv, cwd=None, env=None):
    """Run a tool with both streams captured by communicate(), so a full pipe cannot deadlock."""
    done = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(argv[:4])} exited {done.returncode}: {done.stderr[-600:]}")
    return done.stdout


def listing(tool, scope):
    """Functions a cargo-mutants listing names, keyed by file, name and start, with body spans."""
    argv = ["cargo", "mutants"] if tool == "cargo" else [tool, "mutants"]
    functions, bodies = set(), {}
    for mutant in json.loads(run(argv + ["--list", "--json", "--no-config", *scope]) or "[]"):
        function = mutant.get("function")
        if not isinstance(function, dict):
            continue
        start = function["span"]["start"]
        key = (mutant["file"], function["function_name"], start["line"], start["column"])
        functions.add(key)
        if mutant.get("genre") == "FnValue":
            bodies[key] = mutant["span"]
    return functions, bodies


def neutralised(text, spans, replacement):
    """`text` with each body span (1-based lines, 1-based end-exclusive character columns) replaced,
    keeping every line's number and, where the body's first line is long enough, every column."""
    lines = text.split("\n")
    for span in sorted(spans, key=lambda s: (s["start"]["line"], s["start"]["column"]), reverse=True):
        l1, c1 = span["start"]["line"] - 1, span["start"]["column"] - 1
        l2, c2 = span["end"]["line"] - 1, span["end"]["column"] - 1
        if l1 == l2:
            body = lines[l1][c1:c2]
        else:
            body = "\n".join([lines[l1][c1:], *lines[l1 + 1:l2], lines[l2][:c2]])
        blank = re.sub(r"[^\n]", " ", body)
        fits = len(blank.split("\n", 1)[0]) >= len(replacement)
        new = replacement + (blank[len(replacement):] if fits else blank)
        lines[l1:l2 + 1] = (lines[l1][:c1] + new + lines[l2][c2:]).split("\n")
    return "\n".join(lines)


def canonical_digest(path):
    """sha256 of MIR text with allocation numbers renumbered by first appearance: rustc numbers
    allocations in interning order, which incremental reuse changes (measured)."""
    numbers = {}
    with open(path, encoding="utf-8", errors="replace") as mir:
        text = re.sub(r"\balloc(\d+)\b",
                      lambda m: f"alloc#{numbers.setdefault(m[1], len(numbers))}", mir.read())
    return hashlib.sha256(text.encode()).hexdigest()


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
                                  cwd=self.tree))
        members = [p for p in metadata["packages"] if p["name"] == package]
        if len(members) != 1:
            raise RuntimeError(f"package {package} is not exactly one workspace member")
        self.manifest = members[0]["manifest_path"]

    def digest(self, spans=(), replacement=None, touch=None):
        """Canonical non-test MIR digests per target, or None when an edited tree does not build."""
        for path in self.edited:
            shutil.copyfile(os.path.join(self.source, path), os.path.join(self.tree, path))
        self.edited = set()
        self.count += 1
        by_file = {}
        for path, span in spans:
            by_file.setdefault(path, []).append(span)
        for path, file_spans in by_file.items():
            with open(os.path.join(self.source, path), encoding="utf-8") as original:
                text = neutralised(original.read(), file_spans, replacement)
            with open(os.path.join(self.tree, path), "w", encoding="utf-8") as out:
                out.write(text)
            self.edited.add(path)
        if touch:
            os.utime(os.path.join(self.tree, touch))
        # Incremental sessions reorder interned allocations (measured: fresh and incremental builds
        # of one tree print them in different orders), so every recompile here is a full one.
        env = dict(os.environ, CARGO_ENCODED_RUSTFLAGS="--emit=link,mir", CARGO_INCREMENTAL="0")
        env.pop("CARGO_TARGET_DIR", None)
        try:
            output = run(["cargo", "test", "--workspace", "--no-run", "--locked", "--message-format=json"],
                         cwd=self.tree, env=env)
        except RuntimeError:
            if spans:
                return None
            raise
        deps = os.path.join(self.tree, "target", "debug", "deps")
        tests, libraries, bins = set(), set(), set()
        for line in output.splitlines():
            message = json.loads(line) if line.startswith("{") else {}
            if message.get("reason") != "compiler-artifact" or message.get("manifest_path") != self.manifest:
                continue
            # Library outputs live in deps/ as lib<stem>.<ext>; `cdylib` and `dylib` carry no hash.
            stems = {os.path.basename(f).split(".")[0].removeprefix("lib")
                     for f in message["filenames"] if os.path.dirname(f) == deps}
            kinds = set(message["target"]["kind"])
            if message["profile"]["test"]:
                tests |= stems | {os.path.basename(f) for f in message["filenames"]}
            elif kinds & LIBRARY_KINDS:
                libraries |= stems
            elif "bin" in kinds:
                bins.add(message["target"]["name"].replace("-", "_"))
        mirs = {}
        for stem in sorted(libraries):
            mirs[f"lib {stem}"] = [os.path.join(deps, f"{stem}.mir")]
        for name in sorted(bins):
            # An uplifted bin names no hash: its MIR is the deps/<name>-<hash>.mir no test owns.
            mirs[f"bin {name}"] = [path for path in glob.glob(os.path.join(deps, f"{name}-*.mir"))
                                   if os.path.basename(path)[:-4] not in tests | libraries]
        digests = {}
        for label, chosen in mirs.items():
            if len(chosen) != 1 or not os.path.exists(chosen[0]):
                raise RuntimeError(f"cannot identify the non-test MIR of {label}: {chosen}")
            digests[label] = canonical_digest(chosen[0])
        if not digests:
            raise RuntimeError(f"the gate build produced no non-test lib or bin of {self.package}")
        return digests


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
    verdict["reproducible"] = build.digest(touch=hidden[0][0]) == original
    if not verdict["reproducible"]:
        verdict["reasons"].append("two builds of the unmodified tree disagree: the comparison cannot be trusted")
    # The control comes from the whole package when the scope itself mutates nothing.
    if not stock_bodies:
        stock, stock_bodies = listing("cargo", ["--package", package])
    verdict["control"] = {"mir_changed": False}
    for control in sorted(key for key in stock if key in stock_bodies)[:CONTROL_TRIES]:
        changed = build.digest([(control[0], stock_bodies[control])], REPLACEMENTS[0])
        if changed is not None:
            verdict["control"] = {"file": control[0], "function": control[1], "mir_changed": changed != original}
            break
    if not verdict["control"]["mir_changed"]:
        verdict["reasons"].append("no mutated function both built and changed the MIR when neutralised")

    def ships(spans):
        return any(build.digest(spans, replacement) != original for replacement in REPLACEMENTS)

    neutralisable = [key for key in hidden if key in bodies]
    shipped = ships([(key[0], bodies[key]) for key in neutralisable])
    if shipped:
        named = []
        if len(neutralisable) <= ATTRIBUTION_LIMIT:
            named = [key for key in neutralisable if ships([(key[0], bodies[key])])]
        verdict["refused"] = [{"file": f, "function": n, "line": line} for f, n, line, _ in named] or [
            {"file": None, "function": f"{len(neutralisable)} hidden functions, not individually named", "line": None}]
    verdict["builds"] = build.count
    verdict["state"] = "inconclusive" if verdict["reasons"] else ("refused" if shipped else "passed")
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
        print(f"refused: {row['file']}:{row['line']} {row['function']}", file=sys.stderr)
    return {"passed": 0, "refused": 1}.get(verdict["state"], 2)


if __name__ == "__main__":
    sys.exit(main())
