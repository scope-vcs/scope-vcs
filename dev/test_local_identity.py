import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
IDENTITY = ROOT / "dev/local-identity.py"
PROCESS = ROOT / "dev/local-process.py"
SERVER = """
import http.server, sys
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
    def log_message(self, *args):
        pass
server = http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
print("ready", flush=True)
server.serve_forever()
"""
SECRET = "postgres://scope:hunter2-secret@127.0.0.1/scope_dev"
GIT = {"GIT_AUTHOR_NAME": "Scope Test", "GIT_AUTHOR_EMAIL": "scope@example.test",
       "GIT_COMMITTER_NAME": "Scope Test", "GIT_COMMITTER_EMAIL": "scope@example.test"}


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class LocalIdentityTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="scope-local-identity-")
        self.addCleanup(temp.cleanup)
        self.path = Path(temp.name).resolve()
        self.root = self.path / "checkout"
        (self.root / "web").mkdir(parents=True)
        (self.root / ".gitignore").write_text(".tmp/\n")
        (self.root / "app.txt").write_text("one\n")
        self.git("init", "-q", "-b", "main")
        self.commit("first")
        self.state = self.root / ".tmp/local-dev"
        self.binary = self.path / "target/debug/api"
        self.build_binary()
        self.ports = {name: free_port() for name in ("api", "worker", "media", "media-worker", "web")}
        (self.path / "bin").mkdir()
        self.serve_config = self.path / "serve.json"
        (self.path / "bin/tailscale").write_text(f'#!/bin/sh\ncat "{self.serve_config}"\n')
        (self.path / "bin/tailscale").chmod(0o755)

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True, text=True,
                              env={**os.environ, **GIT}).stdout.strip()

    def commit(self, message):
        self.git("add", "--all")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def build_binary(self):
        self.binary.parent.mkdir(parents=True, exist_ok=True)
        self.binary.unlink(missing_ok=True)
        shutil.copyfile(sys.executable, self.binary)
        self.binary.chmod(0o755)

    def identity(self, *args, check=True):
        result = subprocess.run(["python3", str(IDENTITY), *map(str, args)], capture_output=True, text=True,
                                timeout=30, env={**os.environ, "PATH": f"{self.path / 'bin'}:{os.environ['PATH']}"})
        if check and result.returncode != 0:
            self.fail(result.stdout + result.stderr)
        return result

    def build(self, change_during_build=None):
        self.identity("build-start", self.root, self.state)
        if change_during_build:
            change_during_build()
        self.identity("build-finish", self.root, self.state, f"api={self.binary}", "web")

    def serve(self, name, executable, cwd, env, record=True):
        server = subprocess.Popen([str(executable), "-c", SERVER, str(self.ports[name])], cwd=cwd,
                                  stdout=subprocess.PIPE, text=True, start_new_session=True,
                                  env={"PATH": os.environ["PATH"], **env})
        self.addCleanup(self.stop, server)
        self.assertEqual(server.stdout.readline().strip(), "ready")
        if record:
            subprocess.run(["python3", str(PROCESS), "record", str(self.state / f"{name}.pid"), str(server.pid)],
                           check=True)
        return server

    def stop(self, server):
        try:
            os.killpg(server.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        server.wait(timeout=5)
        server.stdout.close()

    def start_stack(self, signed_out="1"):
        api = self.serve("api", self.binary, self.root, {"CLERK_ISSUER": "https://dev-1.clerk.accounts.dev",
                                                   "SCOPE_DEV_USER_EMAIL": "seed@example.test",
                                                   "DATABASE_URL": SECRET})
        self.serve("web", sys.executable, self.root / "web", {"VITE_SCOPE_WEB_FORCE_SIGNED_OUT": signed_out})
        return api

    def arguments(self, private=()):
        ports = [f"--port={name}={port}" for name, port in self.ports.items()]
        return ["--root", self.root, "--state", self.state, *ports, *(f"--private={entry}" for entry in private)]

    def verify(self, *change, private=()):
        return self.identity("verify", *self.arguments(private), *change, check=False)

    def status(self):
        return self.identity("status", *self.arguments()).stdout

    def test_verification_requires_the_running_build_to_contain_the_change(self):
        first = self.git("rev-parse", "HEAD")
        self.build()
        self.start_stack(signed_out="0")

        output = self.status()
        self.assertIn(f"running build: {first[:12]}, built ", output)
        self.assertIn("build vs checkout: the running build matches the checkout", output)
        self.assertIn("api: pid ", output)
        self.assertIn(f"runs {first[:12]}; auth Clerk tokens from dev-1.clerk.accounts.dev; HTTP 200", output)
        self.assertIn(f"serves {self.root / 'web'} live from the working tree; auth Clerk sign-in", output)
        self.assertIn("actor: sign in as seed@example.test", output)
        self.assertNotIn("hunter2", output)
        passed = self.verify()
        self.assertEqual(passed.returncode, 0, passed.stdout)
        self.assertIn(f"verified: {first[:12]} from {self.root}", passed.stdout)

        (self.root / "app.txt").write_text("two\n")
        self.assertIn("the uncommitted changes differ from the running build", self.verify().stdout)
        second = self.commit("second")
        stale = self.verify()
        self.assertEqual(stale.returncode, 1)
        self.assertIn("the stack does not serve the checkout: HEAD is 1 commits ahead of the running build",
                      stale.stdout)
        self.assertEqual(self.verify(first).returncode, 0)
        missing = self.verify(second)
        self.assertIn(f"the running build at {first[:12]} does not contain {second[:12]}", missing.stdout)
        self.assertNotIn("the checkout at", missing.stdout)

        self.git("checkout", "-q", "--orphan", "unrelated")
        self.commit("unrelated")
        self.assertIn("the running build is not an ancestor of HEAD", self.verify().stdout)

    def test_a_stale_record_and_another_checkouts_listener_are_not_this_stack(self):
        self.build()
        api = self.start_stack()
        self.stop(api)
        foreign = self.path / "other-checkout"
        foreign.mkdir()
        self.serve("api", sys.executable, foreign, {}, record=False)

        output = self.status()
        self.assertIn(f"api: not this stack, port {self.ports['api']} is served by pid ", output)
        self.assertIn(f"from {foreign} running ", output)
        self.serve_config.write_text(json.dumps({"Web": {
            "stack.invalid:443": {"Handlers": {"/": {"Proxy": f"http://127.0.0.1:{self.ports['media']}"}}},
            "stack.invalid:8443": {"Handlers": {"/": {"Proxy": f"http://127.0.0.1:{self.ports['api']}"}}}}}))
        failed = self.verify(private=["web=https://stack.invalid", "api=https://stack.invalid:8443",
                                      f"media=http://localhost:{self.ports['media']}"])
        self.assertEqual(failed.returncode, 1)
        self.assertIn("  api: stale launcher record", failed.stdout)
        self.assertIn(f"  private web: https://stack.invalid reaches port {self.ports['media']}, "
                      f"not this stack's port {self.ports['web']}", failed.stdout)
        self.assertIn(f"  private api: https://stack.invalid:8443 proxies to this stack's port {self.ports['api']} "
                      "but is unreachable", failed.stdout)
        self.assertIn(f"  private media: http://localhost:{self.ports['media']} reaches an unknown target",
                      failed.stdout)

    def test_a_binary_replaced_after_the_build_is_not_the_built_revision(self):
        self.build()
        self.build_binary()
        self.start_stack()

        failed = self.verify()
        self.assertEqual(failed.returncode, 1)
        self.assertIn("  api: the running binary is not the one up built", failed.stdout)

    def test_a_checkout_that_changes_during_the_build_has_an_unknown_revision(self):
        self.build(change_during_build=lambda: (self.root / "app.txt").write_text("edited\n"))
        self.start_stack()

        self.assertIn("running build: unknown: the checkout changed while up was building", self.status())
        failed = self.verify()
        self.assertEqual(failed.returncode, 1)
        self.assertIn("the running revision is unknown", failed.stdout)


if __name__ == "__main__":
    unittest.main()
