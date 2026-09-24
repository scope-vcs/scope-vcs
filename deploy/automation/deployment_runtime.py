"""GitHub and local T3 adapters for release supervision."""
from __future__ import annotations

import json
import os
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPOSITORY = "scope-vcs/scope-vcs"
CHECKOUT = Path.home() / "code/scope-vcs"
T3_HOME = Path.home() / ".t3"
PROJECT_ID = "62235978-ccf8-4b75-b773-0a5375f2d330"
SELECTIONS = {
    "codex": {"instanceId": "codex", "model": "gpt-6-astra",
              "options": [{"id": "reasoningEffort", "value": "high"}]},
    "claudeAgent": {"instanceId": "claudeAgent", "model": "claude-fable-5-1",
                    "options": [{"id": "effort", "value": "medium"},
                                {"id": "contextWindow", "value": "200k"}]},
}


def save_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_suffix(".tmp")
    with temporary.open("w") as output:
        output.write(json.dumps(value, indent=2) + "\n")
        output.flush()
        os.fsync(output.fileno())
    temporary.replace(path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def github(path: str) -> dict | list:
    result = subprocess.run(["gh", "api", f"repos/{REPOSITORY}/{path}"],
                            capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise RuntimeError("GitHub API request failed")
    return json.loads(result.stdout)


def jobs(run: dict) -> list[dict]:
    result = []
    page = 1
    while True:
        batch = github(f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100&page={page}")["jobs"]
        result.extend(batch)
        if len(batch) < 100:
            return result
        page += 1


class RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        # A redirect must never carry the local session token to another server.
        return None


class T3Client:
    def __enter__(self):
        runtime = json.loads((T3_HOME / "userdata/server-runtime.json").read_text())
        self.origin = runtime["origin"]
        url = urllib.parse.urlparse(self.origin)
        if url.scheme != "http" or url.hostname not in ("127.0.0.1", "localhost", "::1"):
            raise ValueError("Expected a local T3 server")
        version = json.loads((T3_HOME / "runtime/service-state.json").read_text())["activeVersion"]
        if not isinstance(version, str) or not version or version in {".", ".."} or Path(version).name != version:
            raise ValueError("Invalid T3 runtime version")
        self.cli = T3_HOME / "runtime/versions" / version / "t3"
        result = subprocess.run([str(self.cli), "auth", "session", "issue", "--base-dir", str(T3_HOME),
                                 "--ttl", "5m", "--label", "Scope release supervisor", "--json"],
                                capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise RuntimeError("Cannot authenticate to local T3")
        self.session = json.loads(result.stdout)
        return self

    def __exit__(self, *args):
        result = subprocess.run([str(self.cli), "auth", "session", "revoke", "--base-dir", str(T3_HOME),
                                 self.session["sessionId"]],
                                capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError("Cannot revoke local T3 session")

    def request(self, path: str, data: dict | None = None) -> dict:
        request = urllib.request.Request(
            self.origin + path, data=None if data is None else json.dumps(data).encode(),
            headers={"Authorization": "Bearer " + self.session["token"], "Content-Type": "application/json"})
        try:
            with urllib.request.build_opener(RejectRedirects()).open(request, timeout=45) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            # Provider responses and authentication diagnostics must not enter alerts.
            raise RuntimeError(f"T3 request failed with HTTP {error.code}") from None

    def dispatch(self, command: dict) -> dict:
        return self.request("/api/orchestration/dispatch", command)

    def thread(self, thread_id: str) -> dict:
        return self.request("/api/orchestration/threads/" + thread_id)["thread"]


def create_worktree(target: Path) -> str:
    def git(*args):
        result = subprocess.run(["git", "-C", str(CHECKOUT), *args],
                                capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError("Release worktree operation failed")
        return result.stdout.strip()
    # Existing work is never reset, including after a lost dispatch response.
    if target.exists():
        root = subprocess.check_output(["git", "-C", str(target), "rev-parse", "--show-toplevel"], text=True).strip()
        common = subprocess.check_output(["git", "-C", str(target), "rev-parse", "--path-format=absolute", "--git-common-dir"], text=True).strip()
        if Path(root).resolve() != target.resolve() or common != git("rev-parse", "--path-format=absolute", "--git-common-dir"):
            raise RuntimeError("Unexpected release worktree")
        return subprocess.check_output(["git", "-C", str(target), "rev-parse", "HEAD"], text=True).strip()
    git("fetch", "origin", "main")
    head = git("rev-parse", "refs/remotes/origin/main^{commit}")
    target.parent.mkdir(parents=True, exist_ok=True)
    git("worktree", "add", "--detach", str(target), head)
    return head
