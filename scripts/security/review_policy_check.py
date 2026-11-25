#!/usr/bin/env python3
# Review council: Compatibility entry point for CI and humans; the checker itself ships pinned in the review crate.
# Design/status: docs/REVIEW_COUNCIL.md#parsed-antipattern-guard
"""Run the pinned review-policy checker against this repository root.

Council gates never execute this wrapper: they run the controller's embedded copy, so a
candidate cannot grade its own policy check.

The wrapper adds one check of this repository that the pinned checker does not make, since it
concerns a package outside the workspace: SECURITY.md says `unsafe_code = "forbid"` holds for
every first-party workspace crate "and the separate fuzz package". The fuzz package is excluded
from the workspace and cannot inherit its lints, so `fuzz/Cargo.toml` must forbid `unsafe_code`
in its own `[lints.rust]` (0.11.1, lane T2: before, `unsafe_code = "allow"` there passed).
`--selftest` runs the pinned checker's controls and this check's (`FuzzManifestControls`).
"""
import json
import os
import pathlib
import runpy
import sys
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
PINNED = ROOT / "crates/deep-diff-forge-review/policy/review_policy_check.py"
FUZZ_MANIFEST = "fuzz/Cargo.toml"


def fuzz_lint_errors(root):
    """The errors of `root`'s fuzz manifest: it must exist, parse, and forbid `unsafe_code`."""
    path = pathlib.Path(root) / FUZZ_MANIFEST
    try:
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return [f"{FUZZ_MANIFEST} is missing: SECURITY.md names the separate fuzz package"]
    except (tomllib.TOMLDecodeError, UnicodeDecodeError, OSError) as error:
        return [f"{FUZZ_MANIFEST} cannot be read: {error}"]
    rust = manifest.get("lints", {}).get("rust", {})
    value = rust.get("unsafe_code") if isinstance(rust, dict) else None
    level = value.get("level") if isinstance(value, dict) else value
    if level != "forbid":
        return [f"{FUZZ_MANIFEST}: lints.rust.unsafe_code must be forbid, found {level!r}"]
    return []


def run_pinned():
    """Run the pinned checker as its own `__main__`; True when it failed."""
    try:
        runpy.run_path(str(PINNED), run_name="__main__")
    except SystemExit as exit:
        if isinstance(exit.code, str):
            print(exit.code, file=sys.stderr)
            return True
        return exit.code not in (None, 0, False)
    return False


class FuzzManifestControls(unittest.TestCase):
    FORBID = '[package]\nname = "fuzz"\n\n[lints.rust]\nunsafe_code = "forbid"\n'

    def check(self, text):
        with tempfile.TemporaryDirectory() as root:
            if text is not None:
                (pathlib.Path(root) / "fuzz").mkdir()
                (pathlib.Path(root) / FUZZ_MANIFEST).write_text(text, encoding="utf-8")
            return fuzz_lint_errors(root)

    def test_forbid_passes(self):
        self.assertEqual(self.check(self.FORBID), [])
        table = self.FORBID.replace('"forbid"', '{ level = "forbid", priority = 0 }')
        self.assertEqual(self.check(table), [])

    def test_a_weakened_or_absent_lint_refuses(self):
        for planted in [
            self.FORBID.replace('"forbid"', '"allow"'),
            self.FORBID.replace('"forbid"', '"deny"'),
            self.FORBID.replace('"forbid"', '"warn"'),
            self.FORBID.replace('"forbid"', '{ level = "allow", priority = 0 }'),
            self.FORBID.replace('unsafe_code = "forbid"', 'unused = "forbid"'),
            self.FORBID.replace('[lints.rust]\nunsafe_code = "forbid"\n', '[lints]\nworkspace = true\n'),
            '[package]\nname = "fuzz"\n',
        ]:
            errors = self.check(planted)
            self.assertTrue(errors and "unsafe_code must be forbid" in errors[0], (planted, errors))

    def test_a_missing_or_unreadable_manifest_refuses(self):
        self.assertIn("is missing", self.check(None)[0])
        self.assertIn("cannot be read", self.check("[lints.rust\n")[0])

    def test_the_repository_manifest_passes_and_its_weakening_refuses(self):
        self.assertEqual(fuzz_lint_errors(ROOT), [])
        text = (ROOT / FUZZ_MANIFEST).read_text(encoding="utf-8")
        self.assertEqual(text.count('unsafe_code = "forbid"'), 1)
        errors = self.check(text.replace('unsafe_code = "forbid"', 'unsafe_code = "allow"'))
        self.assertEqual(errors, [f"{FUZZ_MANIFEST}: lints.rust.unsafe_code must be forbid, found 'allow'"])


def main():
    os.chdir(ROOT)
    failed = run_pinned()
    if sys.argv[1:] == ["--selftest"]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(FuzzManifestControls)
        failed |= not unittest.TextTestRunner().run(suite).wasSuccessful()
    elif not sys.argv[1:]:
        errors = fuzz_lint_errors(ROOT)
        print(json.dumps({"schema": "deep-diff-forge.fuzz-manifest-lints.v1", "manifest": FUZZ_MANIFEST, "state": "failed" if errors else "passed", "errors": errors}, sort_keys=True))
        failed |= bool(errors)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
