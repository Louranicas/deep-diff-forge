#!/usr/bin/env python3
"""Compare two `deep-diff-forge bench --json` receipts from the same machine.

    scripts/bench_compare.py --base B1.json [B2.json...] --head H1.json [H2.json...]
                             [--max-regression 0.25] [--noise-floor-ns 200000]

Each side may give several receipts (alternating runs on one runner); each workload's best
(lowest) median on that side is compared, so one slow round of noise does not fail a change.

A workload regresses when HEAD's median is slower than BASE's by more than the ratio *and* by more
than the noise floor (so a 30% change of 50 microseconds is not a failure). Workloads in only one
receipt are reported and do not fail. A workload whose input changed (its `input_sha256` differs
between base and head) measured different bytes: it is reported as not compared and never as ok
or slow. Receipts whose profile or host (CPU count, OS and architecture only: two machines that
agree on those are not told apart) differ are refused, as are receipts on one side whose inputs
differ. Exit 0 when nothing regressed, 1 when something did, 2 on a usage or input error,
including a malformed receipt. Plan 2026-10-30, M2.
"""

from __future__ import annotations

import argparse
import json
import sys


def load(path: str) -> dict:
    with open(path, encoding="utf-8") as f:
        receipt = json.load(f)
    if not isinstance(receipt, dict) or receipt.get("schema") != "deep-diff-forge.bench.v0":
        raise ValueError(f"{path} is not a deep-diff-forge.bench.v0 receipt")
    workloads = receipt.get("workloads")
    if not isinstance(workloads, list):
        raise ValueError(f"{path}: workloads is not a list")
    for w in workloads:
        if not (
            isinstance(w, dict)
            and isinstance(w.get("name"), str)
            and isinstance(w.get("input_sha256"), str)
            and type(w.get("median_ns")) is int
            and w["median_ns"] >= 0
        ):
            raise ValueError(f"{path}: a workload lacks a name, an input_sha256 or a non-negative integer median_ns")
    return receipt


def best_of(receipts: list[dict]) -> dict:
    """One receipt holding each workload's lowest median across `receipts` (same profile and host)."""
    first = receipts[0]
    for other in receipts[1:]:
        for key in ("profile", "host"):
            if other.get(key) != first.get(key):
                raise ValueError(f"receipts on one side differ in {key}: not comparable")
    best: dict[str, dict] = {}
    for receipt in receipts:
        for w in receipt["workloads"]:
            kept = best.setdefault(w["name"], {"name": w["name"], "input_sha256": w["input_sha256"], "median_ns": w["median_ns"]})
            if kept["input_sha256"] != w["input_sha256"]:
                raise ValueError(f"receipts on one side measured different inputs for {w['name']}: not comparable")
            kept["median_ns"] = min(kept["median_ns"], w["median_ns"])
    return {**first, "workloads": list(best.values())}


def compare(base: dict, head: dict, max_regression: float, noise_floor_ns: int) -> tuple[list[str], list[str], list[str]]:
    """(regressions, notes, not compared) between two receipts."""
    for key in ("profile", "host"):
        if base.get(key) != head.get(key):
            raise ValueError(f"the receipts differ in {key} ({base.get(key)} vs {head.get(key)}): not comparable")
    olds = {w["name"]: w for w in base["workloads"]}
    regressions, notes, skipped = [], [], []
    for w in head["workloads"]:
        name, now = w["name"], w["median_ns"]
        if name not in olds:
            notes.append(f"{name}: new workload (median {now} ns)")
            continue
        old = olds.pop(name)
        if old["input_sha256"] != w["input_sha256"]:
            skipped.append(f"{name}: input changed ({old['median_ns']} ns -> {now} ns on different bytes)")
            continue
        before = old["median_ns"]
        ratio = now / before if before else float("inf") if now else 1.0
        line = f"{name}: {before} ns -> {now} ns ({ratio:.2f}x)"
        if now - before > noise_floor_ns and ratio > 1 + max_regression:
            regressions.append(line)
        else:
            notes.append(line)
    notes.extend(f"{name}: workload removed" for name in olds)
    return regressions, notes, skipped


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", nargs="+", required=True)
    parser.add_argument("--head", nargs="+", required=True)
    parser.add_argument("--max-regression", type=float, default=0.25)
    parser.add_argument("--noise-floor-ns", type=int, default=200_000)
    args = parser.parse_args(argv)
    try:
        base = best_of([load(p) for p in args.base])
        head = best_of([load(p) for p in args.head])
        regressions, notes, skipped = compare(base, head, args.max_regression, args.noise_floor_ns)
    except (OSError, ValueError, KeyError) as err:
        print(f"bench_compare: {err}", file=sys.stderr)
        return 2
    for line in notes:
        print(f"ok   {line}")
    for line in skipped:
        print(f"SKIP {line}")
    for line in regressions:
        print(f"SLOW {line}")
    print(f"{len(regressions)} regression(s), {len(skipped)} not compared, max {args.max_regression:.0%} and {args.noise_floor_ns} ns noise floor")
    return 1 if regressions else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
