#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`, together with everything inside such an
item. A candidate reaches those exclusions in spellings no syntactic scan predicts: an imported macro
shadowing the built-in `#[test]`, a build script printing `cargo:rustc-cfg=test`, an attribute macro
that renames or re-spans the item, or one on an enclosing module (council r12).

This oracle asks the compiler whether the hidden code ships, by changing it and watching the output:

1. List the scope twice: with the gate's cargo-mutants and with `ddf-cargo-mutants-noexclude`, the
   same pinned crate built with attribute and `unsafe fn` exclusions switched off. A function only
   the second lists was hidden by an exclusion.
2. Copy the snapshot and replace each hidden function's body (cargo-mutants' own body span) with a
   line-preserving `loop {}`. Build the package's production (non-test) MIR from both trees with the
   stable `--emit=mir` and the workspace's unified features, the configuration the mutation gate
   tests. No unstable flag or `RUSTC_BOOTSTRAP` is involved.
3. Identical MIR: none of the hidden code is compiled into production (genuine tests), so pass.
   Different MIR: hidden code ships, so refuse, naming each function whose body alone changes it.
   A macro cannot hide a changed body by renaming or re-spanning it.
4. Positive control: neutralising one function the gate does mutate must change the MIR, or the
   comparison proves nothing and the run is inconclusive.

The controller embeds this file and runs it in the gate sandbox. Candidate build scripts and proc
macros run during the builds, so a candidate that detects this job and behaves differently, or that
forges compiler output, is the documented "lying to the tool" residual (docs/REVIEW_COUNCIL.md).
Exit status: 0 passed, 1 refused, 2 inconclusive; the witness says which.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

NOEXCLUDE = "ddf-cargo-mutants-noexclude"
SCHEMA = "deep-diff-forge.exemption-oracle.v2"
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
# Individually re-built when the batch shows hidden code ships; beyond this, refuse without naming.
ATTRIBUTION_LIMIT = 32


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


def neutralise(tree, spans):
    """Replace each body span (1-based lines, 1-based end-exclusive character columns) with
    `loop {}`, blanking the rest so every later line keeps its number."""
    by_file = {}
    for path, span in spans:
        by_file.setdefault(path, []).append(span)
    for path, file_spans in by_file.items():
        target = os.path.join(tree, path)
        with open(target, encoding="utf-8") as source:
            lines = source.read().split("\n")
        order = sorted(file_spans, key=lambda s: (s["start"]["line"], s["start"]["column"]), reverse=True)
        for span in order:
            l1, c1 = span["start"]["line"] - 1, span["start"]["column"] - 1
            l2, c2 = span["end"]["line"] - 1, span["end"]["column"] - 1
            if l1 == l2:
                body = lines[l1][c1:c2]
            else:
                body = "\n".join([lines[l1][c1:], *lines[l1 + 1:l2], lines[l2][:c2]])
            blanked = lines[l1][:c1] + "loop {}" + re.sub(r"[^\n]", " ", body) + lines[l2][c2:]
            lines[l1:l2 + 1] = blanked.split("\n")
        with open(target, "w", encoding="utf-8") as out:
            out.write("\n".join(lines))


class Builds:
    """Production MIR digests of one package's lib and bin targets, per neutralised copy."""

    def __init__(self, source, scratch, package):
        self.source, self.scratch, self.package = source, scratch, package
        metadata = json.loads(run(["cargo", "metadata", "--locked", "--format-version=1"], cwd=source))
        members = [p for p in metadata["packages"]
                   if p["name"] == package and p["id"] in metadata["workspace_members"]]
        if len(members) != 1:
            raise RuntimeError(f"package {package} is not exactly one workspace member")
        node = next(n for n in metadata["resolve"]["nodes"] if n["id"] == members[0]["id"])
        # The features the workspace build (and so the mutation gate's test build) enables here.
        self.features = sorted(node.get("features", []))
        self.targets = []
        for target in members[0]["targets"]:
            kinds = set(target["kind"])
            if kinds & LIBRARY_KINDS:
                self.targets.append((target["name"], ["--lib"]))
            elif "bin" in kinds and set(target.get("required-features", [])) <= set(self.features):
                self.targets.append((target["name"], ["--bin", target["name"]]))
        if not self.targets:
            raise RuntimeError(f"package {package} has no lib or buildable bin target")
        self.count = 0

    def digest(self, spans):
        """MIR digests per target. A neutralised copy that no longer builds (a `const fn` evaluated
        at compile time, an `impl Trait` inferred from the body) is a change: code absent from the
        production build cannot break it. Only the unmodified tree must build."""
        self.count += 1
        tree = os.path.join(self.scratch, f"tree-{self.count}")
        shutil.copytree(self.source, tree, symlinks=True, ignore=shutil.ignore_patterns("target"))
        neutralise(tree, spans)
        digests = {}
        # A lib and a bin may share a name, so each target's output is numbered, never named.
        for index, (name, selector) in enumerate(self.targets):
            emit = os.path.join(self.scratch, f"{self.count}-{index}.mir")
            try:
                run(
                    ["cargo", "rustc", "--locked", "-q", "-p", self.package, *selector,
                     "--no-default-features", "--features", ",".join(self.features),
                     "--", f"--emit=mir={emit}"],
                    cwd=tree,
                    env=dict(os.environ, CARGO_TARGET_DIR=os.path.join(self.scratch, "target")),
                )
            except RuntimeError:
                if not spans:
                    raise
                digests[(index, name)] = "build failed"
                continue
            with open(emit, "rb") as mir:
                digests[(index, name)] = hashlib.sha256(mir.read()).hexdigest()
        shutil.rmtree(tree)
        return digests


def control_function(package, stock, stock_bodies):
    """A function the gate mutates, from the scope or else the whole package, with its body."""
    candidates = sorted(key for key in stock if key in stock_bodies)
    if candidates:
        return candidates[0], stock_bodies[candidates[0]]
    package_stock, package_bodies = listing("cargo", ["--package", package])
    candidates = sorted(key for key in package_stock if key in package_bodies)
    return (candidates[0], package_bodies[candidates[0]]) if candidates else (None, None)


def judge(package, in_diff, scratch):
    scope = ["--package", package] + (["--in-diff", in_diff] if in_diff else [])
    stock, stock_bodies = listing("cargo", scope)
    examined, bodies = listing(NOEXCLUDE, scope)
    hidden = sorted(examined - stock)
    verdict = {"schema": SCHEMA, "package": package, "in_diff": bool(in_diff),
               "hidden_functions": len(hidden), "builds": 0, "refused": [], "reasons": []}
    if not hidden:
        verdict["state"] = "passed"
        return verdict
    builds = Builds(os.getcwd(), scratch, package)
    verdict["features"] = builds.features
    verdict["targets"] = [name for name, _ in builds.targets]
    unbodied = [key for key in hidden if key not in bodies]
    if unbodied:
        verdict["reasons"].append(f"{len(unbodied)} hidden functions have no body to neutralise: {unbodied[:5]}")
    original = builds.digest([])
    control, control_body = control_function(package, stock, stock_bodies)
    if control is None:
        verdict["reasons"].append("no mutated function in the package to serve as the positive control")
    else:
        changed = builds.digest([(control[0], control_body)]) != original
        verdict["control"] = {"file": control[0], "function": control[1], "mir_changed": changed}
        if not changed:
            verdict["reasons"].append("neutralising a mutated function left the MIR unchanged: the comparison is blind")
    neutralisable = [key for key in hidden if key in bodies]
    shipped = builds.digest([(key[0], bodies[key]) for key in neutralisable]) != original
    if shipped:
        named = []
        if len(neutralisable) <= ATTRIBUTION_LIMIT:
            named = [key for key in neutralisable if builds.digest([(key[0], bodies[key])]) != original]
        verdict["refused"] = [{"file": f, "function": n, "line": line} for f, n, line, _ in named] or [
            {"file": None, "function": f"{len(neutralisable)} hidden functions, not attributed", "line": None}]
    verdict["builds"] = builds.count
    verdict["state"] = "inconclusive" if verdict["reasons"] else ("refused" if shipped else "passed")
    return verdict


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", required=True)
    parser.add_argument("--witness", required=True)
    parser.add_argument("--scratch", required=True)
    parser.add_argument("--in-diff")
    args = parser.parse_args()
    try:
        verdict = judge(args.package, args.in_diff, args.scratch)
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
