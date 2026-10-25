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


class ProbeTests(unittest.TestCase):
    def check_status(self, code, stdout):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            completed = subprocess.CompletedProcess([], code, stdout, "failure" if code else "")
            with patch.object(probe, "REPO", root), patch.object(probe, "REPORT_DIR", root), patch.object(probe, "run", return_value=completed), patch.object(pathlib.Path, "exists", return_value=True):
                result = probe.main()
            report = json.loads(next(root.glob("learning-privacy-probe-*.json")).read_text())
            return result, report["verdict"]

    def test_command_failure_is_failure(self):
        self.assertEqual(self.check_status(3, ""), (1, "FAIL"))

    def test_malformed_json_is_failure(self):
        self.assertEqual(self.check_status(0, "not JSON"), (1, "FAIL"))

    def test_wrong_shape_is_failure(self):
        for value in [[], {}, {"schema": "wrong", "scores": []}]:
            self.assertEqual(self.check_status(0, json.dumps(value)), (1, "FAIL"))

    def test_empty_valid_status_passes(self):
        self.assertEqual(self.check_status(0, json.dumps({"schema": "deep-diff-forge.learning.v0", "scores": []})), (0, "PASS"))


if __name__ == "__main__":
    unittest.main()
