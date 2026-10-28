#!/usr/bin/env python3
# DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
# Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fscripts--security--test_privacy_probe
"""Regression tests: a failed or malformed probe must never generate PASS (R58)."""
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("privacy_probe", pathlib.Path(__file__).with_name("privacy_probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


GOOD_STATUS = {
    "schema": "deep-diff-forge.learning.v0",
    "store": "/tmp/ddf-privacy-probe-x/ddf-learning",
    "total_receipts": 1,
    "trusted_default": None,
    "scores": [{"strategy": "syntax", "samples": 1, "trusted": False}],
}


class ProbeTests(unittest.TestCase):
    def test_build_failure_is_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            failed = subprocess.CompletedProcess([], 101, "", "build failed")
            with patch.object(probe, "REPO", root), patch.object(probe, "REPORT_DIR", root), patch.object(probe, "run", return_value=failed):
                result = probe.main()
            report = json.loads(next(root.glob("learning-privacy-probe-*.json")).read_text())
        self.assertEqual((result, report["verdict"]), (1, "FAIL"))

    def test_malformed_or_misshapen_status_is_failure(self):
        for stdout in ["not JSON", "[]", "{}", json.dumps({"schema": "wrong", "scores": []})]:
            self.assertIsNotNone(probe.check_status(stdout), stdout)

    def test_an_empty_store_is_failure_not_a_vacuous_pass(self):
        empty = {**GOOD_STATUS, "total_receipts": 0, "scores": []}
        self.assertIn("does not score", probe.check_status(json.dumps(empty)))

    def test_leaked_keys_or_values_in_status_are_failure(self):
        keyed = {**GOOD_STATUS, "scores": [{"strategy": "syntax", "path": "x"}]}
        self.assertIn("sensitive keys", probe.check_status(json.dumps(keyed)))
        valued = {**GOOD_STATUS, "trusted_default": probe.PATH_SENTINEL}
        self.assertIn("leaks", probe.check_status(json.dumps(valued)))

    def test_the_store_location_itself_is_not_a_leak(self):
        located = {**GOOD_STATUS, "store": probe.PATH_SENTINEL}
        self.assertIsNone(probe.check_status(json.dumps(located)))

    def test_stored_receipts_hold_only_receipt_fields(self):
        good = probe.receipt()
        self.assertIsNone(probe.check_store(good + "\n"))
        self.assertIn("exactly one", probe.check_store(""))
        self.assertIn("exactly one", probe.check_store(good + "\n" + good + "\n"))
        self.assertIn("keys", probe.check_store(probe.receipt(path="x") + "\n"))
        self.assertIn("leak", probe.check_store(probe.receipt(language=probe.SOURCE_SENTINEL) + "\n"))
        self.assertIn("hex", probe.check_store(probe.receipt(file_hash="NOT-HEX") + "\n"))


if __name__ == "__main__":
    unittest.main()
