"""Tests for the Python client against the real binary: DDF_BIN names it (default: target/debug).

Run: DDF_BIN=target/debug/deep-diff-forge python3 -I sdk/python/test_deep_diff_forge.py
"""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import deep_diff_forge as ddf  # noqa: E402

REPO = pathlib.Path(__file__).resolve().parents[2]
BIN = os.environ.get("DDF_BIN", str(REPO / "target" / "debug" / "deep-diff-forge"))
PATCH = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n"


def cli_review(patch):
    out = subprocess.run([BIN, "--stdin-patch", "--json"], input=patch, capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


class Stdio(unittest.TestCase):
    def test_review_patch_matches_the_cli(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            self.assertEqual(client.review_patch(PATCH), cli_review(PATCH))
            self.assertIn("diff.compute", [m["name"] for m in client.methods()])

    def test_errors_keep_their_codes(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            with self.assertRaises(ddf.RpcError) as refused:
                client.review_patch("", require_files=True)
            self.assertEqual((refused.exception.code, refused.exception.message),
                             (7, "refused: 0 files in input (--require-files)"))
            with self.assertRaises(ddf.RpcError) as unknown:
                client.call("no.such")
            self.assertEqual(unknown.exception.code, -32601)
            self.assertEqual(client.call("daemon.health")["status"], "ok")

    def test_an_oversize_request_is_refused_locally(self):
        with ddf.StdioClient((BIN, "rpc")) as client:
            with self.assertRaises(ValueError):
                client.review_patch("x" * ddf.MAX_REQUEST_BYTES)


class Socket(unittest.TestCase):
    def test_a_live_daemon_answers_like_the_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            os.chmod(directory, 0o700)
            path = pathlib.Path(directory) / "d.sock"
            daemon = subprocess.Popen([BIN, "daemon", "start", "--foreground", "--socket", str(path)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                client = ddf.SocketClient(path, timeout=10)
                deadline = time.monotonic() + 10
                while True:
                    try:
                        client.call("daemon.health")
                        break
                    except OSError:
                        if time.monotonic() > deadline:
                            raise
                        time.sleep(0.05)
                self.assertEqual(client.review_patch(PATCH), cli_review(PATCH))
                with self.assertRaises(ddf.RpcError) as failed:
                    client.review_patch("--- a/x\n+++ b/x\n+stray\n")
                self.assertEqual(failed.exception.code, 4)
                client.call("daemon.shutdown")
                self.assertEqual(daemon.wait(timeout=10), 0)
            finally:
                if daemon.poll() is None:
                    daemon.kill()


if __name__ == "__main__":
    unittest.main(verbosity=1)
