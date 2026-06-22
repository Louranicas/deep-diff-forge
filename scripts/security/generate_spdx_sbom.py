#!/usr/bin/env python3
"""Generate a deterministic SPDX 2.3 JSON SBOM from Cargo metadata.

The project may not have CycloneDX/cargo-sbom installed on every verifier host.
This script is dependency-free and emits an auditable SPDX JSON document from the
same Cargo.lock-resolved graph that `cargo metadata --locked` sees. It is not a
replacement for SLSA provenance or signing; it is the local SBOM artifact gate.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import pathlib
import subprocess
import sys
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[2]
OUTPUT = REPO / "sbom.spdx.json"
SPDX = "SPDXRef-"


def _run_metadata() -> dict[str, Any]:
    cmd = ["cargo", "metadata", "--locked", "--format-version", "1"]
    proc = subprocess.run(cmd, cwd=REPO, text=True, capture_output=True, check=False)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)
    return json.loads(proc.stdout)


def _spdx_id(name: str, version: str) -> str:
    digest = hashlib.sha256(f"{name}@{version}".encode()).hexdigest()[:16]
    safe = "".join(ch if ch.isalnum() or ch in ".-" else "-" for ch in name)
    return f"{SPDX}Package-{safe}-{digest}"


def _external_refs(package: dict[str, Any]) -> list[dict[str, str]]:
    source = package.get("source") or ""
    if not source.startswith("registry+"):
        return []
    purl = f"pkg:cargo/{package['name']}@{package['version']}"
    return [
        {
            "referenceCategory": "PACKAGE-MANAGER",
            "referenceType": "purl",
            "referenceLocator": purl,
        }
    ]


def main() -> int:
    metadata = _run_metadata()
    packages = metadata["packages"]
    root_package_ids = set(metadata.get("workspace_members", []))
    package_by_id = {p["id"]: p for p in packages}

    package_ids: dict[str, str] = {}
    spdx_packages: list[dict[str, Any]] = []
    for pkg in sorted(packages, key=lambda p: (p["name"], p["version"], p["id"])):
        spdx_id = _spdx_id(pkg["name"], pkg["version"])
        package_ids[pkg["id"]] = spdx_id
        source = pkg.get("source") or "NOASSERTION"
        checksums: list[dict[str, str]] = []
        manifest = pkg.get("manifest_path")
        if manifest and pathlib.Path(manifest).is_file():
            checksums.append(
                {
                    "algorithm": "SHA256",
                    "checksumValue": hashlib.sha256(pathlib.Path(manifest).read_bytes()).hexdigest(),
                }
            )
        spdx_packages.append(
            {
                "name": pkg["name"],
                "SPDXID": spdx_id,
                "versionInfo": pkg["version"],
                "downloadLocation": source,
                "filesAnalyzed": False,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": pkg.get("license") or "NOASSERTION",
                "copyrightText": "NOASSERTION",
                "checksums": checksums,
                "externalRefs": _external_refs(pkg),
                "primaryPackagePurpose": "APPLICATION" if pkg["id"] in root_package_ids else "LIBRARY",
            }
        )

    relationships: list[dict[str, str]] = []
    document_id = "SPDXRef-DOCUMENT"
    for member_id in sorted(root_package_ids):
        if member_id in package_ids:
            relationships.append(
                {
                    "spdxElementId": document_id,
                    "relationshipType": "DESCRIBES",
                    "relatedSpdxElement": package_ids[member_id],
                }
            )

    resolve = metadata.get("resolve") or {}
    for node in resolve.get("nodes", []):
        parent = package_ids.get(node["id"])
        if parent is None:
            continue
        for dep in sorted(node.get("deps", []), key=lambda d: d["pkg"]):
            child = package_ids.get(dep["pkg"])
            if child is None or child == parent:
                continue
            relationships.append(
                {
                    "spdxElementId": parent,
                    "relationshipType": "DEPENDS_ON",
                    "relatedSpdxElement": child,
                }
            )

    now = _dt.datetime.now(_dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    doc = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": document_id,
        "name": "deep-diff-forge-cargo-lock-sbom",
        "documentNamespace": f"https://github.com/Louranicas/deep-diff-forge/sbom/{now}",
        "creationInfo": {
            "created": now,
            "creators": ["Tool: deep-diff-forge scripts/security/generate_spdx_sbom.py"],
        },
        "packages": spdx_packages,
        "relationships": relationships,
    }

    OUTPUT.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    print(f"wrote {OUTPUT.relative_to(REPO)} packages={len(spdx_packages)} relationships={len(relationships)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
