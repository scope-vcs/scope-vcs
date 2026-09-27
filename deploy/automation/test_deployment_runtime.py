"""Adapters use temporary credentials and preserve interrupted repair work."""
import contextlib
import json
import os
import stat
import subprocess
import tempfile
import threading
import unittest
import urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

import deployment_runtime as runtime


class StateTests(unittest.TestCase):
    def test_save_json_syncs_contents_then_renamed_directory(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "state" / "intent.json"
            kinds = []
            real_fsync = os.fsync

            def sync(fd):
                mode = os.fstat(fd).st_mode
                kinds.append("file" if stat.S_ISREG(mode) else "directory")
                if stat.S_ISDIR(mode):
                    self.assertEqual(json.loads(path.read_text()), {"date": "2026-09-25"})
                real_fsync(fd)

            with patch.object(runtime.os, "fsync", side_effect=sync):
                runtime.save_json(path, {"date": "2026-09-25"})
            self.assertEqual(kinds, ["file", "directory"])
            self.assertFalse(path.with_suffix(".tmp").exists())


class T3Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.addCleanup(patch.stopall)
        patch.object(runtime, "T3_HOME", self.home).start()
        self.configure()

    def configure(self, origin="http://127.0.0.1:3773", version="1.2.3"):
        for path, value in (("userdata/server-runtime.json", {"origin": origin}),
                            ("runtime/service-state.json", {"activeVersion": version})):
            file = self.home / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(json.dumps(value))

    def test_authentication_is_short_lived_and_revoked_even_on_error(self):
        issued = subprocess.CompletedProcess([], 0, json.dumps({"sessionId": "test-session", "token": "test-secret"}))
        revoked = subprocess.CompletedProcess([], 0, "")
        for error in (False, True):
            with self.subTest(error=error), patch.object(runtime.subprocess, "run", side_effect=[issued, revoked]) as execute:
                with self.assertRaisesRegex(RuntimeError, "test failure") if error else contextlib.nullcontext():
                    with runtime.T3Client() as client:
                        self.assertEqual(client.session["token"], "test-secret")
                        if error:
                            raise RuntimeError("test failure")
                issue, revoke = [call.args[0] for call in execute.call_args_list]
                self.assertEqual(issue[0], str(self.home / "runtime/versions/1.2.3/t3"))
                self.assertEqual(issue[1:4], ["auth", "session", "issue"])
                self.assertEqual(issue[issue.index("--ttl") + 1], "5m")
                self.assertEqual(revoke[1:4], ["auth", "session", "revoke"])
                self.assertEqual(revoke[-1], "test-session")
                self.assertNotIn("test-secret", " ".join(revoke))

    def test_nonlocal_servers_are_rejected_before_issuing_credentials(self):
        for origin in ("https://127.0.0.1:3773", "http://example.com:3773", "http://localhost.evil.invalid", "file:///tmp/t3"):
            with self.subTest(origin=origin):
                self.configure(origin=origin)
                with patch.object(runtime.subprocess, "run") as execute:
                    with self.assertRaises(ValueError):
                        runtime.T3Client().__enter__()
                    execute.assert_not_called()

    def test_runtime_version_cannot_escape_version_directory(self):
        for version in ("../other", "..", ".", "/tmp/t3", "", None):
            with self.subTest(version=version):
                self.configure(version=version)
                with patch.object(runtime.subprocess, "run") as execute:
                    with self.assertRaises(ValueError):
                        runtime.T3Client().__enter__()
                    execute.assert_not_called()

    def test_authentication_errors_do_not_expose_diagnostics(self):
        with patch.object(runtime.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "secret output", "secret error")):
            with self.assertRaisesRegex(RuntimeError, "^Cannot authenticate to local T3$"):
                runtime.T3Client().__enter__()

    def test_http_redirects_do_not_forward_bearer_credentials(self):
        received = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                received.append((self.path, self.headers.get("Authorization")))
                if self.path == "/start":
                    self.send_response(302)
                    self.send_header("Location", "/redirected")
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b'{}')

        with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            server_thread = threading.Thread(target=server.serve_forever, daemon=True)
            server_thread.start()
            try:
                client = runtime.T3Client()
                client.origin = f"http://127.0.0.1:{server.server_port}"
                client.session = {"token": "test-secret"}
                with self.assertRaises((RuntimeError, urllib.error.HTTPError)):
                    client.request("/start")
                self.assertEqual(received, [("/start", "Bearer test-secret")])
            finally:
                server.shutdown()
                server_thread.join(timeout=3)


class WorktreeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.checkout = self.root / "checkout"
        self.checkout.mkdir()
        self.git(self.checkout, "init", "-b", "main")
        self.git(self.checkout, "config", "user.email", "test@example.invalid")
        self.git(self.checkout, "config", "user.name", "Test")
        (self.checkout / "tracked").write_text("original\n")
        self.git(self.checkout, "add", "tracked")
        self.git(self.checkout, "commit", "-m", "fixture")
        self.git(self.checkout, "remote", "add", "origin", str(self.checkout))
        self.target = self.root / "repair"
        self.addCleanup(patch.stopall)
        patch.object(runtime, "CHECKOUT", self.checkout).start()

    def git(self, checkout, *args):
        return subprocess.check_output(["git", "-C", str(checkout), *args], stderr=subprocess.DEVNULL, text=True).strip()

    def test_retry_preserves_dirty_tracked_and_untracked_repair_files(self):
        revision = runtime.create_worktree(self.target)
        (self.target / "tracked").write_text("unfinished repair\n")
        (self.target / "notes").write_text("recovery context\n")
        # Main can advance while the same repair is resumed.
        (self.checkout / "tracked").write_text("new main\n")
        self.git(self.checkout, "commit", "-am", "advance main")
        self.assertNotEqual(self.git(self.checkout, "rev-parse", "HEAD"), revision)
        self.assertEqual(runtime.create_worktree(self.target), revision)
        self.assertEqual((self.target / "tracked").read_text(), "unfinished repair\n")
        self.assertEqual((self.target / "notes").read_text(), "recovery context\n")

    def test_unrelated_checkout_cannot_be_reused_as_repair_worktree(self):
        self.target.mkdir()
        self.git(self.target, "init", "-b", "main")
        with self.assertRaisesRegex(RuntimeError, "Unexpected release worktree"):
            runtime.create_worktree(self.target)


class GitHubTests(unittest.TestCase):
    def test_jobs_use_current_attempt_and_read_all_pages(self):
        first_page = [{"name": str(index)} for index in range(100)]
        with patch.object(runtime, "github", side_effect=[{"jobs": first_page}, {"jobs": [{"name": "last"}]}]) as request:
            result = runtime.jobs({"id": 42, "run_attempt": 3})
        self.assertEqual(len(result), 101)
        self.assertEqual([call.args[0] for call in request.call_args_list], [
            "actions/runs/42/attempts/3/jobs?per_page=100&page=1",
            "actions/runs/42/attempts/3/jobs?per_page=100&page=2"])

    def test_github_error_does_not_expose_cli_diagnostics(self):
        with patch.object(runtime.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "secret output", "secret error")):
            with self.assertRaisesRegex(RuntimeError, "^GitHub API request failed$"):
                runtime.github("actions/runs")


if __name__ == "__main__":
    unittest.main()
