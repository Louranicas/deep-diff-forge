#!/usr/bin/env python3
"""Regressions for the soak's leak checks (scripts/security/daemon_soak.py): each check must
fail on the growth it names, pass on a flat run, and report a short run as unmeasured rather
than passed."""

from __future__ import annotations

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import daemon_soak  # noqa: E402

N = daemon_soak.MIN_GROWTH_SAMPLES


def samples(rss: list[int], fds: list[int]) -> list[dict[str, object]]:
    hostile = [{"iteration": 0, "alive": True, "vm_rss": "999999 kB", "fd_count": 99}]
    return hostile + [
        {"iteration": i + 1, "alive": True, "vm_rss": f"{r} kB", "fd_count": f}
        for i, (r, f) in enumerate(zip(rss, fds))
    ]


class Growth(unittest.TestCase):
    def test_a_flat_run_shows_no_growth(self) -> None:
        self.assertEqual(
            daemon_soak.growth(samples([5000] * N, [7] * N)),
            {"samples": N, "rss_growth_kib": 0, "fd_growth": 0},
        )

    def test_rss_that_climbs_is_measured_from_after_the_warm_up(self) -> None:
        # The warm-up window (the first GROWTH_WINDOW samples) is ignored, so a high start does
        # not hide a later climb, and the hostile samples (iteration 0) are never counted.
        rss = [90000] * daemon_soak.GROWTH_WINDOW + list(range(1000, 1000 + 100 * (N - daemon_soak.GROWTH_WINDOW), 100))
        measured = daemon_soak.growth(samples(rss, [7] * N))
        assert measured is not None
        self.assertEqual(measured["rss_growth_kib"], 100 * (N - 2 * daemon_soak.GROWTH_WINDOW))

    def test_leaked_fds_are_counted(self) -> None:
        fds = [7] * (N - daemon_soak.GROWTH_WINDOW) + [7 + 5] * daemon_soak.GROWTH_WINDOW
        measured = daemon_soak.growth(samples([5000] * N, fds))
        assert measured is not None
        self.assertEqual(measured["fd_growth"], 5)
        self.assertGreater(measured["fd_growth"], daemon_soak.MAX_FD_GROWTH)

    def test_a_short_run_is_unmeasured_not_passed(self) -> None:
        self.assertIsNone(daemon_soak.growth(samples([5000] * (N - 1), [7] * (N - 1))))

    def test_a_sample_without_rss_does_not_count(self) -> None:
        rows = samples([5000] * N, [7] * N)
        rows[-1]["vm_rss"] = "unknown"
        self.assertIsNone(daemon_soak.growth(rows))


if __name__ == "__main__":
    unittest.main()
