#!/usr/bin/env python3
# DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
# Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fscripts--security--privacy_probe
"""Privacy probe for the L9 learning store and receipt schema.

Asserts that learned receipts carry only redacted IDs and aggregate metrics; raw
source, patch text, and filesystem paths must not appear in StrategyReceipt JSON.
"""

from __future__ import annotations

import datetime as dt
import json
import os
import pathlib
import re
import subprocess
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[2]
REPORT_DIR = REPO / "reports" / "security"
SENSITIVE_KEYS = {"path", "file_path", "absolute_path", "source", "source_text", "patch", "patch_text", "raw_patch", "raw_source"}
RECEIPT_KEYS = {"file_hash", "language", "parser_version", "strategy", "fallback", "elapsed_ms", "bytes", "nodes", "cache", "outcome", "revisited"}
HEX_RE = re.compile(r"^[0-9a-f]{16,64}$")
# Values a leak would carry: a filesystem path and a line of source, each unique to this probe.
PATH_SENTINEL = "/home/privacy-probe/secret_module.rs"
SOURCE_SENTINEL = "fn privacy_probe_secret() { let token = 7; }"
STORE = "deep-diff-forge/learning/receipts/strategy.jsonl"


def run(cmd: list[str], input_text: str | None = None, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=REPO, text=True, input=input_text, capture_output=True, check=False, env=env)


def receipt(**extra: object) -> str:
    base = {
        "file_hash": "0123456789abcdef",
        "language": "rust",
        "parser_version": "privacy-probe",
        "strategy": "syntax",
        "fallback": None,
        "elapsed_ms": 1,
        "bytes": 42,
        "nodes": 7,
        "cache": "cold",
        "outcome": "accepted",
        "revisited": False,
    }
    base.update(extra)
    return json.dumps(base)


def leaks(text: str) -> list[str]:
    return [name for name, value in (("path", PATH_SENTINEL), ("source", SOURCE_SENTINEL)) if value in text]


def check_store(text: str) -> str | None:
    """The stored JSONL holds exactly one receipt with only the receipt fields and no leaked value."""
    lines = [line for line in text.splitlines() if line.strip()]
    if len(lines) != 1:
        return f"expected exactly one stored receipt, found {len(lines)}"
    try:
        stored = json.loads(lines[0])
    except ValueError as error:
        return f"stored receipt is not JSON: {error}"
    if not isinstance(stored, dict) or set(stored) != RECEIPT_KEYS:
        return f"stored receipt keys are not the receipt fields: {sorted(stored) if isinstance(stored, dict) else stored!r}"
    if not HEX_RE.fullmatch(str(stored["file_hash"])):
        return "stored file_hash is not redacted hex"
    if leaks(text):
        return f"stored receipts leak {leaks(text)}"
    return None


def check_status(stdout: str) -> str | None:
    """`learn status --json` is the learning schema, scores the recorded receipt, and leaks nothing."""
    try:
        parsed = json.loads(stdout)
    except (ValueError, TypeError) as error:
        return f"invalid learning status JSON: {error}"
    if not isinstance(parsed, dict) or not isinstance(parsed.get("scores"), list):
        return "learning status is not an object with a scores array"
    if parsed.get("schema") != "deep-diff-forge.learning.v0":
        return "unexpected learning status schema"
    if parsed.get("total_receipts") != 1 or not parsed["scores"]:
        # An empty store would make every check below vacuous.
        return f"learning status does not score the one recorded receipt: total_receipts={parsed.get('total_receipts')!r}"
    score_keys: set[str] = set()
    for score in parsed["scores"]:
        if not isinstance(score, dict):
            return "a score is not an object"
        score_keys.update(score)
    forbidden = sorted(score_keys & SENSITIVE_KEYS)
    if forbidden:
        return f"learning status leaks sensitive keys: {forbidden}"
    # `store` is the caller's own store location; everything else must carry no leaked value.
    rest = {key: value for key, value in parsed.items() if key != "store"}
    if leaks(json.dumps(rest)):
        return f"learning status leaks {leaks(json.dumps(rest))}"
    return None


def main() -> int:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).strftime("%Y-%m-%dT%H-%M-%SZ")
    report = {
        "schema": "deep-diff-forge.learning-privacy-probe.v0",
        "ts": stamp,
        "checks": [],
        "verdict": "FAIL",
    }
    # Build every time: cargo is a no-op when the binary is current, and a stale binary would
    # probe old code.
    build = run(["cargo", "build", "-p", "deep-diff-forge-cli"])
    report["checks"].append({"kind": "build", "code": build.returncode})
    if build.returncode != 0:
        report["error"] = build.stderr[-4000:]
        return write(report)
    binary = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "target"))) / "debug" / "deep-diff-forge"

    with tempfile.TemporaryDirectory(prefix="ddf-privacy-probe-") as state:
        env = {**os.environ, "XDG_STATE_HOME": state, "HOME": state}

        # Extra path and source fields are not part of a receipt: they are dropped, never stored.
        carrying = run([str(binary), "learn", "record", "--stdin"], receipt(path=PATH_SENTINEL, source_text=SOURCE_SENTINEL), env)
        report["checks"].append({"kind": "record_with_extra_fields", "code": carrying.returncode})
        if carrying.returncode != 0:
            report["error"] = f"a valid receipt with extra fields was not recorded: {carrying.stderr[-500:]}"
            return write(report)

        # A path smuggled into a stored token field is refused (exit 4) and stores nothing.
        smuggled = run([str(binary), "learn", "record", "--stdin"], receipt(language=PATH_SENTINEL), env)
        report["checks"].append({"kind": "record_path_as_language", "code": smuggled.returncode})
        if smuggled.returncode != 4:
            report["error"] = f"a path in language was not refused as invalid input (exit {smuggled.returncode})"
            return write(report)

        store = pathlib.Path(state) / STORE
        error = check_store(store.read_text() if store.exists() else "")
        report["checks"].append({"kind": "stored_receipts", "ok": error is None})
        if error:
            report["error"] = error
            return write(report)

        status = run([str(binary), "learn", "status", "--json"], env=env)
        report["checks"].append({"kind": "learn_status_json", "code": status.returncode})
        if status.returncode != 0:
            report["learn_status_stderr"] = status.stderr[-1000:]
            report["error"] = "learning status command failed"
            return write(report)
        error = check_status(status.stdout)
        if error:
            report["error"] = error
            return write(report)

    report["verdict"] = "PASS"
    return write(report)


def write(receipt: dict) -> int:
    out = REPORT_DIR / f"learning-privacy-probe-{receipt['ts']}.json"
    out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"receipt={out.relative_to(REPO)} verdict={receipt['verdict']}")
    return 0 if receipt["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
