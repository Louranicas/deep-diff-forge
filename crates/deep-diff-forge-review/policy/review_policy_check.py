#!/usr/bin/env python3
# Review council (pinned in deep-diff-forge-review): Parse workspace TOML and provenance JSON; check lint inheritance and source labels without pretending to detect all semantic antipatterns.
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

# The council embeds this file in the controller and runs it inside the candidate snapshot,
# so the workspace root is the working directory, never this file's own location.
ROOT = pathlib.Path.cwd().resolve()


def unique_object(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def enforcement_errors(row, workspace, clippy):
    """DDF's own enforcement claim on an imported row (anchor B-10), kept beside the verbatim import
    labels. Each piece of evidence must resolve against parsed configuration (AP-26), never text."""
    claim = row.get("ddf_enforcement")
    if claim is None:
        return []
    ident = row.get("id")
    if claim.get("state") != "ENFORCED" or not claim.get("task") or not claim.get("scope") or not claim.get("by"):
        return [f"{ident}: ddf_enforcement needs state ENFORCED, task, scope and evidence"]
    errors = []
    lints = workspace.get("workspace", {}).get("lints", {})
    methods = {entry.get("path") if isinstance(entry, dict) else entry for entry in (clippy or {}).get("disallowed-methods", [])}
    for evidence in claim["by"]:
        kind = evidence.get("kind")
        if kind == "workspace-lint":
            group, _, name = evidence.get("lint", "").partition(".")
            value = lints.get(group, {}).get(name)
            level = value.get("level") if isinstance(value, dict) else value
            if not name or level not in ("deny", "forbid"):
                errors.append(f"{ident}: workspace lint {evidence.get('lint')} is not deny")
        elif kind == "clippy-disallowed-method":
            if evidence.get("path") not in methods:
                errors.append(f"{ident}: clippy.toml does not disallow {evidence.get('path')}")
        else:
            errors.append(f"{ident}: unknown enforcement evidence kind {kind!r}")
    return errors


def inspect(workspace, members, catalogue, clippy=None):
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
        errors.extend(enforcement_errors(row, workspace, clippy))
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
    clippy_toml = ROOT / "clippy.toml"
    clippy = tomllib.loads(clippy_toml.read_text()) if clippy_toml.exists() else None
    errors = inspect(workspace, members, catalogue, clippy)
    provenance = json.loads((ROOT / "review-policy/provenance.json").read_text(), object_pairs_hook=unique_object)
    for entry in provenance["files"]:
        if "destination" in entry:
            digest = hashlib.sha256((ROOT / entry["destination"]).read_bytes()).hexdigest()
            if digest != entry["sha256"]:
                errors.append("pstack reference drift: " + entry["destination"])
    report = {"schema": "deep-diff-forge.antipattern-contracts.v1", "state": "failed" if errors else "passed", "workspace_members_checked": len(members), "advisory_catalogue_entries": len(catalogue), "semantic_detectors_qualified": 0, "ddf_enforced": sorted(row["id"] for row in catalogue if "ddf_enforcement" in row), "errors": errors}
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

    def test_enforcement_claims_must_resolve(self):
        workspace = copy.deepcopy(self.workspace)
        workspace["workspace"]["lints"]["clippy"]["allow_attributes"] = "deny"
        clippy = {"disallowed-methods": [{"path": "std::time::Instant::now", "reason": "door"}]}
        claim = {"state": "ENFORCED", "task": "B-03", "scope": "s", "by": [
            {"kind": "workspace-lint", "lint": "clippy.allow_attributes"},
            {"kind": "clippy-disallowed-method", "path": "std::time::Instant::now"}]}
        row = dict(self.catalogue[0], ddf_enforcement=claim)
        self.assertEqual(inspect(workspace, self.members, [row], clippy), [])
        self.assertTrue(inspect(self.workspace, self.members, [row], clippy), "lint not deny")
        self.assertTrue(inspect(workspace, self.members, [row], {"disallowed-methods": []}), "method not disallowed")
        self.assertTrue(inspect(workspace, self.members, [row], None), "no clippy.toml")
        for broken in [dict(claim, state="PARTIAL"), dict(claim, by=[]), dict(claim, task=""),
                       dict(claim, by=[{"kind": "comment-says-so"}])]:
            self.assertTrue(inspect(workspace, self.members, [dict(row, ddf_enforcement=broken)], clippy), broken)

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
