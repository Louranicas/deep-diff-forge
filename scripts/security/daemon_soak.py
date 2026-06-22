#!/usr/bin/env python3
"""Hostile-client soak for the Deep-Diff-Forge UDS daemon.

The soak uses an explicit socket path under a freshly-created 0700 temp dir,
then exercises healthy JSON-RPC clients, malformed JSON, empty lines, and short
slowloris-style connects. It records resource samples and proves the daemon can
still stop cleanly after adversarial traffic.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import socket
import stat
import subprocess
import sys
import tempfile
import time
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[2]
REPORT_DIR = REPO / "reports" / "security"
PI_REPORT_DIR = REPO / ".pi" / "reports"
BIN = REPO / "target" / "debug" / "deep-diff-forge"


def run(cmd: list[str], *, timeout: int = 30, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=REPO,
        input=input_text,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )


def ensure_binary() -> None:
    if BIN.exists():
        return
    proc = run(["cargo", "build", "-p", "deep-diff-forge-cli"], timeout=240)
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)


def cli(socket_path: pathlib.Path, subcommand: str, *, timeout: int = 10) -> subprocess.CompletedProcess[str]:
    return run([str(BIN), "daemon", subcommand, "--socket", str(socket_path)], timeout=timeout)


def raw_request(socket_path: pathlib.Path, payload: bytes, *, read: bool = True, timeout: float = 2.0) -> bytes:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(timeout)
        sock.connect(str(socket_path))
        if payload:
            sock.sendall(payload)
        if read:
            return sock.recv(4096)
        return b""


def proc_sample(pid: int) -> dict[str, Any]:
    proc = pathlib.Path("/proc") / str(pid)
    status: dict[str, str] = {}
    try:
        for line in (proc / "status").read_text().splitlines():
            if ":" in line:
                key, value = line.split(":", 1)
                status[key] = value.strip()
    except FileNotFoundError:
        return {"alive": False}
    try:
        fd_count = len(list((proc / "fd").iterdir()))
    except OSError:
        fd_count = None
    return {
        "alive": True,
        "vm_rss": status.get("VmRSS", "unknown"),
        "threads": status.get("Threads", "unknown"),
        "fd_count": fd_count,
    }


def start_daemon(socket_path: pathlib.Path) -> subprocess.Popen[str]:
    proc = subprocess.Popen(
        [str(BIN), "daemon", "start", "--socket", str(socket_path)],
        cwd=REPO,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            out, err = proc.communicate(timeout=1)
            raise RuntimeError(f"daemon exited early code={proc.returncode}\nstdout={out}\nstderr={err}")
        if socket_path.exists():
            mode = stat.S_IMODE(socket_path.lstat().st_mode)
            if mode == 0o600:
                return proc
        time.sleep(0.05)
    proc.terminate()
    raise RuntimeError("daemon did not create a 0600 socket before timeout")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=int, default=int(os.environ.get("DDF_SOAK_SECONDS", "60")))
    parser.add_argument("--max-rss-kib", type=int, default=int(os.environ.get("DDF_SOAK_MAX_RSS_KIB", "262144")))
    args = parser.parse_args()
    ensure_binary()
    REPORT_DIR.mkdir(parents=True, exist_ok=True)

    started = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    receipt: dict[str, Any] = {
        "schema": "deep-diff-forge.daemon-soak.v0",
        "started_utc": started.isoformat().replace("+00:00", "Z"),
        "duration_seconds_requested": args.seconds,
        "checks": [],
        "samples": [],
        "verdict": "FAIL",
    }

    with tempfile.TemporaryDirectory(prefix="ddf-soak-") as temp:
        temp_path = pathlib.Path(temp)
        os.chmod(temp_path, 0o700)
        socket_path = temp_path / "daemon.sock"
        proc = start_daemon(socket_path)
        receipt["pid"] = proc.pid
        receipt["socket_mode"] = oct(stat.S_IMODE(socket_path.lstat().st_mode))
        deadline = time.monotonic() + args.seconds
        iteration = 0
        try:
            while time.monotonic() < deadline:
                iteration += 1
                health = cli(socket_path, "health")
                receipt["checks"].append(
                    {"kind": "health", "iteration": iteration, "code": health.returncode, "stdout": health.stdout.strip()[:300]}
                )
                if health.returncode != 0:
                    raise RuntimeError(f"health failed at iteration {iteration}: {health.stderr}")

                status_out = cli(socket_path, "status")
                receipt["checks"].append(
                    {"kind": "status", "iteration": iteration, "code": status_out.returncode, "stdout": status_out.stdout.strip()[:300]}
                )
                if status_out.returncode != 0:
                    raise RuntimeError(f"status failed at iteration {iteration}: {status_out.stderr}")

                malformed = raw_request(socket_path, b"{not-json}\n")
                parsed = json.loads(malformed.decode())
                if "error" not in parsed:
                    raise RuntimeError(f"malformed request did not return error: {parsed}")

                raw_request(socket_path, b"", read=False)
                receipt["samples"].append({"iteration": iteration, **proc_sample(proc.pid)})
                time.sleep(1)

            stop = cli(socket_path, "stop", timeout=10)
            receipt["checks"].append({"kind": "stop", "code": stop.returncode, "stdout": stop.stdout.strip()[:300]})
            if stop.returncode != 0:
                raise RuntimeError(f"stop failed: {stop.stderr}")
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired as exc:
                proc.terminate()
                raise RuntimeError("daemon did not exit after shutdown") from exc
            receipt["exit_code"] = proc.returncode
            rss_values = []
            for sample in receipt["samples"]:
                value = str(sample.get("vm_rss", ""))
                if value.endswith(" kB"):
                    rss_values.append(int(value[:-3].strip()))
            receipt["max_rss_kib"] = max(rss_values) if rss_values else None
            if receipt["max_rss_kib"] is not None and receipt["max_rss_kib"] > args.max_rss_kib:
                raise RuntimeError(f"rss exceeded limit: {receipt['max_rss_kib']} > {args.max_rss_kib}")
            receipt["verdict"] = "PASS"
        except Exception as exc:  # noqa: BLE001 - receipt must capture all soak failures.
            receipt["error"] = str(exc)
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
        finally:
            ended = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
            receipt["ended_utc"] = ended.isoformat().replace("+00:00", "Z")
            receipt["duration_seconds_observed"] = int((ended - started).total_seconds())
            stamp = ended.strftime("%Y-%m-%dT%H-%M-%SZ")
            out = REPORT_DIR / f"daemon-soak-{stamp}.json"
            out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
            PI_REPORT_DIR.mkdir(parents=True, exist_ok=True)
            pi_out = PI_REPORT_DIR / f"{stamp}-daemon-soak.json"
            pi_out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
            print(f"receipt={out.relative_to(REPO)} pi_receipt={pi_out.relative_to(REPO)} verdict={receipt['verdict']}")
            if receipt["verdict"] != "PASS":
                return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
