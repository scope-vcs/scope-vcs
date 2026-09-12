#!/usr/bin/env python3
"""Minimal runtime-protocol fixture for the container entrypoint regression."""

import argparse
import hashlib
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class Fixture:
    def __init__(self, args: argparse.Namespace) -> None:
        self.scenario = args.scenario
        self.bundle = Path(args.bundle).read_bytes()
        self.source_sha256 = hashlib.sha256(self.bundle).hexdigest()
        self.git_oid = args.git_oid
        self.shared = Path(args.shared)
        self.state_path = self.shared / "protocol-state.json"
        self.lock = threading.Lock()
        self.events: list[dict] = []
        self.claims = 0
        self.step_started = False
        self.heartbeat_count = 0
        self.step_completions: list[dict] = []
        self.attempt_completions: list[dict] = []
        self.write_state()

    def write_state(self) -> None:
        state = {
            "scenario": self.scenario,
            "claims": self.claims,
            "step_started": self.step_started,
            "heartbeat_count": self.heartbeat_count,
            "step_completions": self.step_completions,
            "attempt_completions": self.attempt_completions,
            "events": self.events,
        }
        temporary = self.state_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(state, sort_keys=True))
        os.replace(temporary, self.state_path)

    def record(self, method: str, action: str, body: object = None) -> None:
        with self.lock:
            self.events.append({"method": method, "action": action, "body": body})
            if action == "claim":
                self.claims += 1
            elif action == "steps/0/start":
                self.step_started = True
            elif action == "heartbeat":
                self.heartbeat_count += 1
            elif action == "steps/0/complete":
                self.step_completions.append(body)
            elif action == "complete":
                self.attempt_completions.append(body)
            self.write_state()

    def status(self, *, canceled: bool = False) -> dict:
        return {
            "state": "running",
            "cancellation_requested": canceled,
            "lease_expires_at_unix": int(time.time()) + 300,
        }

    def step(self) -> tuple[str, int]:
        process_tree = (
            "sleep 30 & descendant=$!; "
            "printf '%s %s\\n' \"$$\" \"$descendant\" > /scope-fixture/processes; "
            "wait"
        )
        if self.scenario == "success":
            return "printf 'runtime entrypoint success\\n'", 30
        if self.scenario == "step-failure":
            return "printf 'expected step failure\\n' >&2; exit 23", 30
        if self.scenario == "timeout-tree":
            return process_tree, 1
        return process_tree, 30

    def claim_response(self, port: int) -> dict:
        command, timeout_seconds = self.step()
        image = f"fixture.invalid/checks@sha256:{'1' * 64}"
        return {
            "attempt_token": "attempt-token",
            "lease_expires_at_unix": int(time.time()) + 300,
            "cache_endpoint": f"http://127.0.0.1:{port}",
            "cache_grant": "unused-cache-grant",
            "job": {
                "run_id": "container-regression",
                "job_key": "entrypoint",
                "repository_id": "fixture-repository",
                "workflow_path": "/.scope/runs/container-regression.yml",
                "git_oid": self.git_oid,
                "source_digest": "fixture-source",
                "pinned_container_image": image,
                "definition": {
                    "id": "entrypoint",
                    "needs": [],
                    "container": {"image": image},
                    "timeout_seconds": timeout_seconds,
                    "caches": [],
                    "environment": {},
                    "steps": [{"name": self.scenario, "run": command}],
                },
            },
        }


class Handler(BaseHTTPRequestHandler):
    server: "FixtureServer"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def action(self) -> str | None:
        prefix = "/v1/runtime-protocol/attempts/container-regression/"
        if not self.path.startswith(prefix):
            return None
        return self.path.removeprefix(prefix)

    def authorized(self, action: str) -> bool:
        expected = "bootstrap-token" if action == "claim" else "attempt-token"
        return self.headers.get("Authorization") == f"Bearer {expected}"

    def json_response(self, status: int, value: object) -> None:
        encoded = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def empty_response(self, status: int = 204) -> None:
        self.send_response(status)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:
        action = self.action()
        if action != "source" or not self.authorized(action):
            self.empty_response(404 if action != "source" else 401)
            return
        fixture = self.server.fixture
        fixture.record("GET", action)
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(fixture.bundle)))
        self.send_header("x-scope-source-identity", "fixture-source")
        self.send_header("x-scope-source-sha256", fixture.source_sha256)
        self.end_headers()
        self.wfile.write(fixture.bundle)

    def do_POST(self) -> None:
        action = self.action()
        if action is None:
            self.empty_response(404)
            return
        if not self.authorized(action):
            self.empty_response(401)
            return
        length = int(self.headers.get("Content-Length", "0"))
        raw_body = self.rfile.read(length)
        body = json.loads(raw_body) if raw_body else None
        fixture = self.server.fixture
        fixture.record("POST", action, body)

        if action == "claim":
            if fixture.scenario == "claim-error":
                self.empty_response(500)
            else:
                self.json_response(200, fixture.claim_response(self.server.server_port))
            return
        if action == "heartbeat":
            canceled = fixture.scenario == "cancel-tree" and fixture.step_started
            self.json_response(
                200,
                {"status": fixture.status(canceled=canceled), "cache_grant": "unused"},
            )
            return
        if action == "steps/0/start" or action == "steps/0/complete":
            self.json_response(200, fixture.status())
            return
        if action == "logs" or action.startswith("cache-observations/"):
            self.empty_response()
            return
        if action == "complete":
            if fixture.scenario in {"cancel-tree", "timeout-tree"}:
                (fixture.shared / "complete-pending").touch()
                release = fixture.shared / "release-completion"
                deadline = time.monotonic() + 10
                while not release.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
            self.json_response(200, fixture.status())
            return
        if action == "abandon":
            self.empty_response()
            return
        self.empty_response(404)


class FixtureServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, fixture: Fixture) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.fixture = fixture


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--bundle", required=True)
    parser.add_argument("--git-oid", required=True)
    parser.add_argument("--shared", required=True)
    parser.add_argument("--port-file", required=True)
    args = parser.parse_args()
    fixture = Fixture(args)
    server = FixtureServer(fixture)
    Path(args.port_file).write_text(str(server.server_port))
    server.serve_forever()


if __name__ == "__main__":
    main()
