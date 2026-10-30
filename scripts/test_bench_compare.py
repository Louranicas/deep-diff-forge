#!/usr/bin/env python3
"""Regressions for scripts/bench_compare.py: it must fail on a planted slowdown, ignore noise and
changes under the floor, report added and removed workloads, never compare timings taken on
different inputs, refuse receipts from different profiles or hosts, and exit 2 (never the
regression code) on a malformed receipt."""

from __future__ import annotations

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import bench_compare  # noqa: E402

HOST = {"cpus": 16, "os": "linux", "arch": "x86_64"}


def receipt(**medians: int) -> dict:
    return {
        "schema": "deep-diff-forge.bench.v0",
        "profile": "release",
        "host": HOST,
        "workloads": [{"name": n, "input_sha256": f"sha-{n}", "median_ns": m} for n, m in medians.items()],
    }


def with_input(r: dict, name: str, sha: str) -> dict:
    for w in r["workloads"]:
        if w["name"] == name:
            w["input_sha256"] = sha
    return r


class Compare(unittest.TestCase):
    def test_a_planted_slowdown_fails(self) -> None:
        regressions, _, _ = bench_compare.compare(receipt(a=10_000_000), receipt(a=13_000_001), 0.25, 200_000)
        self.assertEqual(regressions, ["a: 10000000 ns -> 13000001 ns (1.30x)"])

    def test_the_ratio_and_the_floor_must_both_be_exceeded(self) -> None:
        # 25% exactly is not over the ratio; a big ratio under the floor is noise.
        self.assertEqual(bench_compare.compare(receipt(a=10_000_000), receipt(a=12_500_000), 0.25, 200_000)[0], [])
        self.assertEqual(bench_compare.compare(receipt(a=50_000), receipt(a=200_000), 0.25, 200_000)[0], [])
        self.assertEqual(len(bench_compare.compare(receipt(a=50_000), receipt(a=300_000), 0.25, 200_000)[0]), 1)

    def test_faster_is_fine_and_workload_changes_are_noted(self) -> None:
        regressions, notes, skipped = bench_compare.compare(receipt(a=10_000_000, gone=5), receipt(a=1_000_000, new=7), 0.25, 0)
        self.assertEqual(regressions, [])
        self.assertIn("new: new workload (median 7 ns)", notes)
        self.assertIn("gone: workload removed", notes)
        self.assertEqual(skipped, [])

    def test_timings_on_different_inputs_are_never_ok_or_slow(self) -> None:
        # A smaller input must not hide a slower engine, and a larger one is not a regression.
        for now in (1_000_000, 30_000_000):
            regressions, notes, skipped = bench_compare.compare(
                receipt(a=10_000_000), with_input(receipt(a=now), "a", "other"), 0.25, 200_000
            )
            self.assertEqual((regressions, notes), ([], []))
            self.assertEqual(skipped, [f"a: input changed (10000000 ns -> {now} ns on different bytes)"])
        with self.assertRaises(ValueError):
            bench_compare.best_of([receipt(a=1), with_input(receipt(a=1), "a", "other")])

    def test_receipts_from_different_machines_are_refused(self) -> None:
        other = receipt(a=1)
        other["host"] = dict(HOST, cpus=4)
        with self.assertRaises(ValueError):
            bench_compare.compare(receipt(a=1), other, 0.25, 0)
        debug = receipt(a=1)
        debug["profile"] = "debug"
        with self.assertRaises(ValueError):
            bench_compare.compare(receipt(a=1), debug, 0.25, 0)

    def test_the_exit_code_follows_the_verdict(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            base, slow, same = (pathlib.Path(d) / n for n in ("base.json", "slow.json", "same.json"))
            base.write_text(json.dumps(receipt(a=10_000_000)))
            slow.write_text(json.dumps(receipt(a=20_000_000)))
            same.write_text(json.dumps(receipt(a=10_000_000)))
            self.assertEqual(bench_compare.main(["--base", str(base), "--head", str(same)]), 0)
            self.assertEqual(bench_compare.main(["--base", str(base), "--head", str(slow)]), 1)
            # One slow round among several is noise: the best median per side is compared.
            self.assertEqual(bench_compare.main(["--base", str(base), "--head", str(slow), str(same)]), 0)
            missing = str(pathlib.Path(d) / "missing.json")
            self.assertEqual(bench_compare.main(["--base", str(base), "--head", missing]), 2)
            # A malformed receipt is an input error, never a regression.
            bad = pathlib.Path(d) / "bad.json"
            nameless = receipt(a=1)
            del nameless["workloads"][0]["name"]
            unhashed = receipt(a=1)
            del unhashed["workloads"][0]["input_sha256"]
            for body in ([], dict(receipt(), workloads=None), dict(receipt(), workloads=[7]),
                         receipt(a=-1), dict(receipt(), workloads=[{"name": "a", "input_sha256": "x", "median_ns": "5"}]),
                         dict(receipt(), workloads=[{"name": "a", "input_sha256": "x", "median_ns": True}]),
                         nameless, unhashed):
                bad.write_text(json.dumps(body))
                self.assertEqual(bench_compare.main(["--base", str(base), "--head", str(bad)]), 2, body)

    def test_the_best_median_per_side_is_kept_in_order(self) -> None:
        best = bench_compare.best_of([receipt(a=5, b=9), receipt(a=3, b=12, c=1)])
        self.assertEqual(
            [(w["name"], w["median_ns"]) for w in best["workloads"]], [("a", 3), ("b", 9), ("c", 1)]
        )
        other = receipt(a=1)
        other["profile"] = "debug"
        with self.assertRaises(ValueError):
            bench_compare.best_of([receipt(a=1), other])


if __name__ == "__main__":
    unittest.main()
