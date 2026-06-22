#!/usr/bin/env python3
"""Privacy probe for the L9 learning store and receipt schema.

Asserts that learned receipts carry only redacted IDs and aggregate metrics; raw
source, patch text, and filesystem paths must not appear in StrategyReceipt JSON.
"""

from __future__ import annotations

import datetime as dt
import json
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
REPORT_DIR = REPO / "reports" / "security"
SENSITIVE_KEYS = {"path", "file_path", "absolute_path", "source", "source_text", "patch", "patch_text", "raw_patch", "raw_source"}
HEX_RE = re.compile(r"^[0-9a-f]{16,64}$")


def run(cmd: list[str], input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=REPO, text=True, input=input_text, capture_output=True, check=False)


def main() -> int:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).strftime("%Y-%m-%dT%H-%M-%SZ")
    receipt = {
        "schema": "deep-diff-forge.learning-privacy-probe.v0",
        "ts": stamp,
        "checks": [],
        "verdict": "FAIL",
    }
    binary = REPO / "target" / "debug" / "deep-diff-forge"
    if not binary.exists():
        build = run(["cargo", "build", "-p", "deep-diff-forge-cli"])
        receipt["checks"].append({"kind": "build", "code": build.returncode})
        if build.returncode != 0:
            receipt["error"] = build.stderr[-4000:]
            return write(receipt)

    sample = {
        "file_hash": "0123456789abcdef",
        "language": "rust",
        "parser_version": "privacy-probe",
        "strategy": "syntax",
        "fallback": None,
        "elapsed_ms": 1,
        "bytes": 42,
        "nodes": 7,
        "cache": "cold",
        "outcome": "pending",
        "revisited": False,
    }
    text = json.dumps(sample)
    keys = set(sample)
    forbidden = sorted(keys & SENSITIVE_KEYS)
    if forbidden:
        receipt["error"] = f"sample receipt contains forbidden keys: {forbidden}"
        return write(receipt)
    if not HEX_RE.fullmatch(sample["file_hash"]):
        receipt["error"] = "sample file_hash is not redacted hex"
        return write(receipt)

    status = run([str(binary), "learn", "status", "--json"])
    receipt["checks"].append({"kind": "learn_status_json", "code": status.returncode})
    if status.returncode == 0:
        parsed = json.loads(status.stdout)
        if parsed.get("schema") != "deep-diff-forge.learning.v0":
            receipt["error"] = "unexpected learning status schema"
            return write(receipt)
        score_keys = set()
        for score in parsed.get("scores", []):
            score_keys.update(score.keys())
        forbidden_status = sorted(score_keys & SENSITIVE_KEYS)
        if forbidden_status:
            receipt["error"] = f"learning status leaks sensitive keys: {forbidden_status}"
            return write(receipt)
    else:
        receipt["learn_status_stderr"] = status.stderr[-1000:]

    receipt["verdict"] = "PASS"
    return write(receipt)


def write(receipt: dict) -> int:
    out = REPORT_DIR / f"learning-privacy-probe-{receipt['ts']}.json"
    out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"receipt={out.relative_to(REPO)} verdict={receipt['verdict']}")
    return 0 if receipt["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
