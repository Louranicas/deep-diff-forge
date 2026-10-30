#!/usr/bin/env python3
# DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
# Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
# Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fscripts--security--daemon_soak
"""Hostile-client soak for the Deep-Diff-Forge UDS daemon.

The soak uses an explicit socket path under a freshly-created 0700 temp dir,
then exercises healthy JSON-RPC clients, malformed JSON, empty lines, short
slowloris-style connects, a request at and over the 80 MiB cap, a flood past the
connection limit and concurrent clients, then real review work from concurrent
clients in every iteration of the timed loop. It records resource samples, checks
RSS and open-fd growth across the loop when the run is long enough to measure it
(--require-growth-check makes a shorter run fail), and proves the daemon can still
stop cleanly after adversarial traffic.
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


WORK_PATCH = (REPO / "fixtures" / "patch" / "basic.patch").read_text()
# Concurrent clients doing real review work in each iteration of the timed loop. Below
# MAX_CONNECTIONS, so the health and status calls of the same iteration still find a slot.
WORK_CLIENTS = 4


def rpc(sock: socket.socket, request_id: int, method: str, params: dict[str, Any]) -> dict[str, Any]:
    """Send one request on an open connection and return its decoded response."""
    line = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
    sock.sendall(line.encode() + b"\n")
    answer = read_line(sock)
    if not answer:
        raise RuntimeError(f"{method}: the daemon closed the connection unanswered")
    response = json.loads(answer)
    if "error" in response or response.get("id") != request_id:
        raise RuntimeError(f"{method}: {str(response)[:300]}")
    return response["result"]


def work_probe(socket_path: pathlib.Path) -> dict[str, Any]:
    """Real review work from concurrent clients: each opens a session on a fixture patch, ranks
    it, closes it, and ranks the patch without a session. Every answer must be right."""
    errors: list[str] = []
    lock = threading.Lock()

    def client() -> None:
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(30)
                sock.connect(str(socket_path))
                opened = rpc(sock, 1, "session.open", {"patch": WORK_PATCH})
                if opened.get("files") != 1 or not isinstance(opened.get("session"), str):
                    raise RuntimeError(f"session.open: {opened}")
                ranked = rpc(sock, 2, "session.snapshot", {"session": opened["session"]})
                if not isinstance(ranked, list) or len(ranked) != 1:
                    raise RuntimeError(f"session.snapshot: {str(ranked)[:300]}")
                closed = rpc(sock, 3, "session.close", {"session": opened["session"]})
                if closed != {"closed": True}:
                    raise RuntimeError(f"session.close: {closed}")
                rpc(sock, 4, "diff.rank", {"patch": WORK_PATCH})
        except (OSError, ValueError, RuntimeError) as exc:
            with lock:
                errors.append(str(exc))

    threads = [threading.Thread(target=client) for _ in range(WORK_CLIENTS)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if errors:
        raise RuntimeError(f"{len(errors)} of {WORK_CLIENTS} work clients failed, first: {errors[0]}")
    return {"kind": "work", "clients": WORK_CLIENTS}


# Leak checks over the timed loop: the median of the last GROWTH_WINDOW samples against the
# median of GROWTH_WINDOW samples taken after a warm-up of GROWTH_WINDOW. Both need at least
# MIN_GROWTH_SAMPLES samples, about a minute of soak; a shorter run records them as unmeasured.
GROWTH_WINDOW = 10
MIN_GROWTH_SAMPLES = 6 * GROWTH_WINDOW
MAX_FD_GROWTH = 4


def median(values: list[int]) -> int:
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def kib(sample: dict[str, Any]) -> int | None:
    value = str(sample.get("vm_rss", ""))
    return int(value[:-3].strip()) if value.endswith(" kB") else None


def growth(samples: list[dict[str, Any]]) -> dict[str, int] | None:
    """RSS and open-fd growth across the timed loop, or None when there are too few samples."""
    loop = [s for s in samples if s.get("iteration", 0) > 0 and s.get("alive")]
    rss = [v for v in (kib(s) for s in loop) if v is not None]
    fds = [s["fd_count"] for s in loop if isinstance(s.get("fd_count"), int)]
    if min(len(rss), len(fds)) < MIN_GROWTH_SAMPLES:
        return None
    early = slice(GROWTH_WINDOW, 2 * GROWTH_WINDOW)
    late = slice(-GROWTH_WINDOW, None)
    return {
        "samples": min(len(rss), len(fds)),
        "rss_growth_kib": median(rss[late]) - median(rss[early]),
        "fd_growth": median(fds[late]) - median(fds[early]),
    }


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
    parser.add_argument(
        "--max-rss-growth-kib", type=int, default=int(os.environ.get("DDF_SOAK_MAX_RSS_GROWTH_KIB", "32768"))
    )
    parser.add_argument(
        "--require-growth-check",
        action="store_true",
        help="fail when the run is too short to measure RSS and fd growth (the long soak)",
    )
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
                work = work_probe(socket_path)
                if iteration == 1:
                    receipt["checks"].append({**work, "iteration": iteration, "repeats": "every iteration"})
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
            receipt["work_iterations"] = iteration
            measured = growth(receipt["samples"])
            receipt["growth"] = measured if measured is not None else "unmeasured: run too short"
            if measured is None:
                if args.require_growth_check:
                    raise RuntimeError(f"too few samples for the growth checks (need {MIN_GROWTH_SAMPLES})")
            else:
                if measured["rss_growth_kib"] > args.max_rss_growth_kib:
                    raise RuntimeError(
                        f"rss grew {measured['rss_growth_kib']} KiB over the soak (limit {args.max_rss_growth_kib})"
                    )
                if measured["fd_growth"] > MAX_FD_GROWTH:
                    raise RuntimeError(f"open fds grew by {measured['fd_growth']} over the soak (limit {MAX_FD_GROWTH})")
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
