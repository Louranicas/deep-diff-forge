#!/usr/bin/python3
"""Mutation-exemption oracle for the council mutation gate (DDF, decision record 2026-10-26).

cargo-mutants 27.1.0 skips, by design, any item carrying `#[mutants::skip]`, `#[cfg(test)]`, an
attribute whose path ends in `test`, and every `unsafe fn`. A candidate can reach those exclusions
in spellings no syntactic scan predicts: an imported macro shadowing the built-in `#[test]`, a
build script printing `cargo:rustc-cfg=test`, or an attribute macro that renames the item.

This oracle asks the compiler instead of predicting it:

1. List the scope twice: with the gate's cargo-mutants, and with `ddf-cargo-mutants-noexclude`,
   the same pinned crate built with attribute and `unsafe fn` exclusions switched off. A function
   that only the second lists was hidden by an exclusion.
2. Dump the production (non-test) HIR of every lib and bin target of the package with the gate's
   stable compiler; `RUSTC_BOOTSTRAP` names that crate only, so proc-macro crates stay on stable.
3. Refuse when any production HIR span lies inside a hidden function's source span (which includes
   its attributes): that code ships, so it must be mutated. Genuine tests are absent from the
   non-test build and pass.

The controller embeds this file and runs it in the gate sandbox. Candidate build scripts and proc
macros run while the HIR is produced, so the result is a sanity check against declarative
exemption, not proof against a candidate that forges compiler output (docs/REVIEW_COUNCIL.md,
residual risks). Exit status: 0 passed, 1 refused, 2 inconclusive; the witness says which.
"""
import argparse
import json
import os
import re
import subprocess
import sys

NOEXCLUDE = "ddf-cargo-mutants-noexclude"
# A rustc span is `<path>:<line>:<col>: <line>:<col> (#<ctxt>)`. The path is matched by that suffix,
# never by its extension: `#[path]` modules and Cargo target roots need not end in `.rs` (council r10).
SPAN = re.compile(r"(\S+?):(\d+):(\d+): (\d+):(\d+) \(#")
SCHEMA = "deep-diff-forge.exemption-oracle.v1"


def listing(argv):
    """The functions a cargo-mutants listing names: (file, name, start line/col, end line/col)."""
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{argv[0]} --list exited {done.returncode}: {done.stderr[-400:]}")
    functions = set()
    for mutant in json.loads(done.stdout or "[]"):
        function = mutant.get("function")
        if not isinstance(function, dict):
            continue
        span = function["span"]
        functions.add(
            (
                mutant["file"],
                function["function_name"],
                span["start"]["line"],
                span["start"]["column"],
                span["end"]["line"],
                span["end"]["column"],
            )
        )
    return functions


def production_spans(package, target, kind):
    """Every source span in the target's non-test HIR, by file, streamed from the compiler."""
    selector = ["--lib"] if kind == "lib" else ["--bin", target["name"]]
    env = dict(os.environ, RUSTC_BOOTSTRAP=target["name"].replace("-", "_"))
    argv = ["cargo", "rustc", "--locked", "-q", "-p", package, *selector, "--", "-Zunpretty=hir-tree"]
    spans = {}
    with subprocess.Popen(
        argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env
    ) as child:
        for line in child.stdout:
            for path, l1, c1, l2, c2 in SPAN.findall(line):
                spans.setdefault(path, []).append((int(l1), int(c1), int(l2), int(c2)))
        stderr = child.stderr.read()
    if child.returncode != 0:
        raise RuntimeError(f"HIR dump of {target['name']} exited {child.returncode}: {stderr[-400:]}")
    return spans


def inside(spans, function):
    path, _, l1, c1, l2, c2 = function
    return [s for s in spans.get(path, ()) if (s[0], s[1]) >= (l1, c1) and (s[2], s[3]) <= (l2, c2)]


def changed_lines(in_diff):
    """New-side numbers of the added lines in each file, whatever the diff's context width."""
    changed, path, number = {}, None, 0
    with open(in_diff, encoding="utf-8", errors="replace") as diff:
        for line in diff:
            if line.startswith("+++ "):
                path = line[6:].rstrip("\n") if line.startswith("+++ b/") else None
                continue
            hunk = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@", line)
            if hunk:
                number = int(hunk[1])
            elif line.startswith("+") and path:
                changed.setdefault(path, set()).add(number)
                number += 1
            elif line.startswith(" "):
                number += 1
    return changed


def unexamined(spans, examined, in_diff):
    """Advisory: changed lines where production code starts outside every function cargo-mutants
    examines even with exclusions off (`const` items, `fn new`, macro and `include!` bodies)."""
    if not in_diff:
        return []
    rows = set()
    for path, lines in changed_lines(in_diff).items():
        functions = [f for f in examined if f[0] == path]
        for span in spans.get(path, ()):
            if span[0] in lines and not any(f[2] <= span[0] <= f[4] for f in functions):
                rows.add((path, span[0]))
    return [{"file": path, "line": line} for path, line in sorted(rows)]


def judge(package, in_diff):
    scope = ["--list", "--json", "--no-config", "--package", package]
    if in_diff:
        scope += ["--in-diff", in_diff]
    stock = listing(["cargo", "mutants", *scope])
    examined = listing([NOEXCLUDE, "mutants", *scope])
    hidden = sorted(examined - stock)
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
            capture_output=True, text=True, check=True,
        ).stdout
    )
    spans, targets, reasons = {}, [], []
    for member in metadata["packages"]:
        if member["name"] != package:
            continue
        for target in member["targets"]:
            kind = "lib" if {"lib", "proc-macro"} & set(target["kind"]) else (
                "bin" if "bin" in target["kind"] else None)
            if kind is None:
                continue
            found = production_spans(package, target, kind)
            if not found:
                reasons.append(f"target {target['name']} dumped no production spans")
            for path, rows in found.items():
                spans.setdefault(path, []).extend(rows)
            targets.append(target["name"])
    if not targets:
        reasons.append(f"package {package} has no lib or bin target to dump")
    covered = [f for f in stock if inside(spans, f)]
    if stock and not covered:
        reasons.append("no function the gate mutates has a production span: the HIR dump is not this code")
    refused = []
    for function in hidden:
        found = inside(spans, function)
        if found:
            refused.append(
                {"file": function[0], "function": function[1], "line": function[2], "production_spans": len(found)}
            )
    state = "inconclusive" if reasons else ("refused" if refused else "passed")
    advisory = unexamined(spans, examined, in_diff)
    return {
        "schema": SCHEMA,
        "state": state,
        "package": package,
        "in_diff": bool(in_diff),
        "targets_dumped": len(targets),
        "stock_functions": len(stock),
        "stock_functions_with_production_spans": len(covered),
        "hidden_functions": len(hidden),
        "refused": refused,
        "reasons": reasons,
        "unexamined_changed_lines": len(advisory),
        "unexamined": advisory[:200],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", required=True)
    parser.add_argument("--witness", required=True)
    parser.add_argument("--in-diff")
    args = parser.parse_args()
    try:
        verdict = judge(args.package, args.in_diff)
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        verdict = {"schema": SCHEMA, "state": "inconclusive", "package": args.package,
                   "targets_dumped": 0, "refused": [], "reasons": [str(error)[-1000:]]}
    with open(args.witness, "w", encoding="utf-8") as out:
        json.dump(verdict, out, indent=1)
    print(json.dumps({k: v for k, v in verdict.items() if k != "refused"} | {"refused": len(verdict["refused"])}))
    for row in verdict["refused"][:50]:
        print(f"refused: {row['file']}:{row['line']} {row['function']}", file=sys.stderr)
    return {"passed": 0, "refused": 1}.get(verdict["state"], 2)


if __name__ == "__main__":
    sys.exit(main())
