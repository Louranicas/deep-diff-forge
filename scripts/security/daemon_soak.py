#!/usr/bin/env python3
# DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
# Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fscripts--security--daemon_soak
"""Hostile-client soak for the Deep-Diff-Forge UDS daemon.

The soak uses an explicit socket path under a freshly-created 0700 temp dir,
then exercises healthy JSON-RPC clients, malformed JSON, empty lines, short
slowloris-style connects, a request at and over the 80 MiB cap, a flood past the
connection limit and concurrent clients. It records resource samples and proves
the daemon can still stop cleanly after adversarial traffic.
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
import threading
import time
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[2]
REPORT_DIR = REPO / "reports" / "security"
PI_REPORT_DIR = REPO / ".pi" / "reports"
# The daemon's limits (crates/deep-diff-forge-daemon/src/serve.rs).
MAX_REQUEST_BYTES = 80 * 1024 * 1024
MAX_CONNECTIONS = 8
HEALTH = b'{"jsonrpc":"2.0","id":7,"method":"daemon.health"}'
BIN = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "target"))) / "debug" / "deep-diff-forge"


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
    # Build every time: cargo is a no-op when the binary is current, and a stale binary would
    # soak old code.
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


def read_line(sock: socket.socket) -> bytes:
    """Read until a newline or end of stream."""
    data = b""
    while not data.endswith(b"\n"):
        chunk = sock.recv(65536)
        if not chunk:
            break
        data += chunk
    return data


def padded_health(total: int, *, newline: bool) -> bytes:
    """A health request padded with spaces to exactly `total` bytes."""
    body = HEALTH.ljust(total - int(newline), b" ")
    return body + b"\n" if newline else body


def send_whole(socket_path: pathlib.Path, payload: bytes, *, timeout: float = 30.0) -> bytes:
    """Send `payload`, close the write side and read one response line."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(timeout)
        sock.connect(str(socket_path))
        sock.sendall(payload)
        sock.shutdown(socket.SHUT_WR)
        return read_line(sock)


def answer_or_closed(socket_path: pathlib.Path, payload: bytes) -> bytes:
    """One response line, or b"" when the daemon closed the connection unanswered (an empty
    read, or a reset or broken pipe if it closed before reading what was sent)."""
    try:
        return send_whole(socket_path, payload, timeout=10)
    except (ConnectionResetError, BrokenPipeError):
        return b""


def oversize_probe(socket_path: pathlib.Path) -> dict[str, Any]:
    """A request of exactly the cap is served; one byte more is refused and the connection closed."""
    at_cap = json.loads(send_whole(socket_path, padded_health(MAX_REQUEST_BYTES, newline=True)))
    if at_cap.get("result", {}).get("status") != "ok":
        raise RuntimeError(f"a request at the {MAX_REQUEST_BYTES}-byte cap was not served: {str(at_cap)[:300]}")
    over = json.loads(send_whole(socket_path, padded_health(MAX_REQUEST_BYTES + 1, newline=False)))
    message = over.get("error", {}).get("message", "")
    if "exceeds maximum size" not in message:
        raise RuntimeError(f"an over-cap request was not refused: {str(over)[:300]}")
    return {"kind": "oversize", "at_cap": "served", "over_cap": message}


def flood_probe(socket_path: pathlib.Path) -> dict[str, Any]:
    """Past the connection limit a connection is closed unanswered; slots free when holders leave."""
    holders = []
    try:
        for _ in range(MAX_CONNECTIONS):
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.settimeout(5)
            sock.connect(str(socket_path))
            holders.append(sock)
        # Each holder proves its worker is running, so all slots are taken.
        for sock in holders:
            sock.sendall(HEALTH + b"\n")
            if b'"ok"' not in read_line(sock):
                raise RuntimeError("a connection under the limit was not served")
        extra = answer_or_closed(socket_path, HEALTH + b"\n")
        if extra:
            raise RuntimeError(f"connection {MAX_CONNECTIONS + 1} was served past the limit: {extra[:200]!r}")
    finally:
        for sock in holders:
            sock.close()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if b'"ok"' in answer_or_closed(socket_path, HEALTH + b"\n"):
            return {"kind": "flood", "held": MAX_CONNECTIONS, "extra": "closed unanswered", "recovered": True}
        time.sleep(0.05)
    raise RuntimeError("the daemon did not serve again after the flood ended")


def concurrent_probe(socket_path: pathlib.Path, *, clients: int = 4 * MAX_CONNECTIONS, rounds: int = 25) -> dict[str, Any]:
    """Many clients at once: every answer is a valid response, and connections past the limit
    are closed unanswered, never answered wrongly."""
    counts = {"served": 0, "closed": 0}
    bad: list[str] = []
    lock = threading.Lock()

    def client() -> None:
        for _ in range(rounds):
            answer = answer_or_closed(socket_path, HEALTH + b"\n")
            with lock:
                if not answer:
                    counts["closed"] += 1
                elif json.loads(answer).get("result", {}).get("status") == "ok":
                    counts["served"] += 1
                else:
                    bad.append(answer[:200].decode(errors="replace"))

    threads = [threading.Thread(target=client) for _ in range(clients)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if bad:
        raise RuntimeError(f"{len(bad)} concurrent answers were wrong, first: {bad[0]}")
    if counts["served"] == 0:
        raise RuntimeError("no concurrent request was served")
    return {"kind": "concurrent", "clients": clients, "rounds": rounds, **counts}


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


def start_daemon(socket_path: pathlib.Path, log_path: pathlib.Path) -> subprocess.Popen[str]:
    # The daemon writes to a file, not a pipe nobody drains: a full pipe would stall it and
    # read as a soak failure of the daemon (bug hunt 2026-10-28, H5-09).
    log = log_path.open("w")
    proc = subprocess.Popen(
        [str(BIN), "daemon", "start", "--socket", str(socket_path)],
        cwd=REPO,
        text=True,
        stdout=log,
        stderr=subprocess.STDOUT,
    )
    log.close()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"daemon exited early code={proc.returncode}\noutput={log_path.read_text()[-2000:]}")
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
        proc = start_daemon(socket_path, temp_path / "daemon.log")
        receipt["pid"] = proc.pid
        receipt["socket_mode"] = oct(stat.S_IMODE(socket_path.lstat().st_mode))
        deadline = time.monotonic() + args.seconds
        iteration = 0
        try:
            # Hostile volume first, so the timed loop below proves the daemon recovered.
            for probe in (oversize_probe, flood_probe, concurrent_probe):
                receipt["checks"].append(probe(socket_path))
                receipt["samples"].append({"iteration": 0, "after": probe.__name__, **proc_sample(proc.pid)})
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
            # A stopped daemon removes the socket it bound.
            if socket_path.exists():
                raise RuntimeError("the socket remained after the daemon stopped")
            rss_values = []
            for sample in receipt["samples"]:
                value = str(sample.get("vm_rss", ""))
                if value.endswith(" kB"):
                    rss_values.append(int(value[:-3].strip()))
            receipt["max_rss_kib"] = max(rss_values) if rss_values else None
            # No sample means the memory bound was never measured (no /proc, as on macOS), which
            # is not a pass (bug hunt 2026-10-28, H5-09).
            if receipt["max_rss_kib"] is None:
                raise RuntimeError("no RSS sample: the memory bound was not measured")
            if receipt["max_rss_kib"] > args.max_rss_kib:
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
