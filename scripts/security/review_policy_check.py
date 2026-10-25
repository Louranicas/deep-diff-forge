#!/usr/bin/env python3
# Review council: Compatibility entry point for CI and humans; the checker itself ships pinned in the review crate.
# Design/status: docs/REVIEW_COUNCIL.md#parsed-antipattern-guard
"""Run the pinned review-policy checker against this repository root.

Council gates never execute this wrapper: they run the controller's embedded copy, so a
candidate cannot grade its own policy check.
"""
import os
import pathlib
import runpy

ROOT = pathlib.Path(__file__).resolve().parents[2]
os.chdir(ROOT)
runpy.run_path(str(ROOT / "crates/deep-diff-forge-review/policy/review_policy_check.py"), run_name="__main__")
