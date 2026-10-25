#!/usr/bin/env python3
# Review council: Parse workspace TOML and provenance JSON; check lint inheritance and source labels without pretending to detect all semantic antipatterns.
# Design/status: docs/REVIEW_COUNCIL.md#parsed-antipattern-guard
# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--scripts--security--review_policy_check
"""Parsed policy/manifest guard. This is not a semantic detector for every imported AP.

Design: docs/REVIEW_COUNCIL.md; vault: Implementation/Review Council.
AP-26: parse TOML/JSON, never infer Rust declarations from regex.
"""
import copy
import hashlib
import json
import pathlib
import sys
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


def unique_object(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def inspect(workspace, members, catalogue):
    errors = []
    if not members:
        errors.append("empty member inventory cannot qualify")
    lints = workspace.get("workspace", {}).get("lints", {})
    for group, name, expected in [("rust", "unsafe_code", "forbid"), ("clippy", "unwrap_in_result", "deny"), ("clippy", "panic_in_result_fn", "deny")]:
        value = lints.get(group, {}).get(name)
        level = value.get("level") if isinstance(value, dict) else value
        if level != expected:
            errors.append(f"{group}.{name} must be {expected}")
    for name, member in members:
        if member.get("lints", {}).get("workspace") is not True:
            errors.append(f"{name}: missing inherited workspace lint boundary")
    ids = [row.get("id") for row in catalogue]
    if not ids or len(ids) != len(set(ids)):
        errors.append("empty or duplicate AP inventory")
    for row in catalogue:
        if row.get("status") != "SOURCE_REGISTER_PRESENT__HEE5_ENFORCEMENT_UNVERIFIED" or not row.get("authoritative_source") or not row.get("basis"):
            errors.append("imported AP row lost its original evidence/qualification labels")
    return errors


def main():
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    members = []
    for pattern in workspace["workspace"]["members"]:
        paths = sorted(ROOT.glob(pattern))
        if not paths:
            raise ValueError(f"member pattern matches nothing: {pattern}")
        for path in paths:
            manifest = (path / "Cargo.toml").resolve()
            if not manifest.is_relative_to(ROOT):
                raise ValueError("workspace member escapes repository")
            members.append((str(path.relative_to(ROOT)), tomllib.loads(manifest.read_text())))
    catalogue = json.loads((ROOT / "crates/deep-diff-forge-review/policy/antipatterns.json").read_text(), object_pairs_hook=unique_object)
    errors = inspect(workspace, members, catalogue)
    provenance = json.loads((ROOT / "review-policy/provenance.json").read_text(), object_pairs_hook=unique_object)
    for entry in provenance["files"]:
        if "destination" in entry:
            digest = hashlib.sha256((ROOT / entry["destination"]).read_bytes()).hexdigest()
            if digest != entry["sha256"]:
                errors.append("pstack reference drift: " + entry["destination"])
    report = {"schema": "deep-diff-forge.antipattern-contracts.v1", "state": "failed" if errors else "passed", "workspace_members_checked": len(members), "advisory_catalogue_entries": len(catalogue), "semantic_detectors_qualified": 0, "errors": errors}
    print(json.dumps(report, sort_keys=True))
    return 1 if errors else 0


class NegativeControls(unittest.TestCase):
    def setUp(self):
        self.workspace = {"workspace": {"lints": {"rust": {"unsafe_code": "forbid"}, "clippy": {"unwrap_in_result": "deny", "panic_in_result_fn": "deny"}}}}
        self.members = [("crate", {"lints": {"workspace": True}})]
        self.catalogue = [{"id": "AP-01", "status": "SOURCE_REGISTER_PRESENT__HEE5_ENFORCEMENT_UNVERIFIED", "authoritative_source": "pinned", "basis": "unverified"}]

    def test_valid_contract(self):
        self.assertEqual(inspect(self.workspace, self.members, self.catalogue), [])

    def test_missing_lint_inheritance_refuses(self):
        self.assertTrue(inspect(self.workspace, [("crate", {})], self.catalogue))

    def test_lint_weakening_refuses(self):
        for group, name in [("rust", "unsafe_code"), ("clippy", "unwrap_in_result"), ("clippy", "panic_in_result_fn")]:
            planted = copy.deepcopy(self.workspace)
            planted["workspace"]["lints"][group][name] = "allow"
            self.assertTrue(inspect(planted, self.members, self.catalogue))

    def test_empty_inventory_refuses(self):
        self.assertTrue(inspect(self.workspace, [], self.catalogue))
        self.assertTrue(inspect(self.workspace, self.members, []))

    def test_laundered_or_duplicate_catalogue_refuses(self):
        planted = copy.deepcopy(self.catalogue)
        planted[0]["status"] = "ENFORCED"
        self.assertTrue(inspect(self.workspace, self.members, planted))
        self.assertTrue(inspect(self.workspace, self.members, self.catalogue * 2))

    def test_duplicate_json_keys_refuse(self):
        with self.assertRaises(ValueError):
            json.loads('{"state":"hold","state":"pass"}', object_pairs_hook=unique_object)


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        unittest.main(argv=[sys.argv[0]])
    elif sys.argv[1:]:
        raise SystemExit("usage: review_policy_check.py [--selftest]")
    else:
        raise SystemExit(main())
