from __future__ import annotations

import base64
import json
import os
import socket
import struct
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPOSITORY = "scope-vcs/scope-vcs"
CHECKOUT = Path.home() / "code/scope-vcs"
T3_HOME = Path.home() / ".t3"
PROJECT_ID = "62235978-ccf8-4b75-b773-0a5375f2d330"
PROTOCOL_HEADER = "x-t3-orchestration-protocol"
PROTOCOL_VERSION = "2"
SELECTIONS = {
    "codex": {"instanceId": "codex", "model": "gpt-6-astra",
              "options": [{"id": "reasoningEffort", "value": "high"}]},
    "claudeAgent": {"instanceId": "claudeAgent", "model": "claude-opus-5-5",
                    "options": [{"id": "effort", "value": "high"},
                                {"id": "contextWindow", "value": "200k"}]},
}


SERVER_SYSTEM_ACTOR = {"createdBy": "system", "creationSource": "server"}


def thread_create_command(thread_id: str, title: str, worktree: str, provider: str) -> dict:
    return {"type": "thread.create", **SERVER_SYSTEM_ACTOR, "commandId": thread_id + "-create", "threadId": thread_id,
            "projectId": PROJECT_ID, "title": title, "modelSelection": SELECTIONS[provider],
            "runtimeMode": "full-access", "interactionMode": "default", "branch": None,
            "worktreePath": worktree}


def message_command(command_id: str, thread_id: str, text: str, provider: str) -> dict:
    return {"type": "message.dispatch", **SERVER_SYSTEM_ACTOR, "commandId": command_id, "threadId": thread_id,
            "messageId": command_id + "-prompt", "text": text, "attachments": [],
            "modelSelection": SELECTIONS[provider], "dispatchMode": {"type": "queue_after_active"}}


def interrupt_command(command_id: str, thread_id: str, run_id: str) -> dict:
    return {"type": "run.interrupt", "commandId": command_id, "threadId": thread_id, "runId": run_id}


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
        return None


class WebSocket:
    def __init__(self, origin: str, path: str, headers: dict[str, str]):
        url = urllib.parse.urlparse(origin)
        self.socket = socket.create_connection((url.hostname, url.port), timeout=45)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f"GET {path} HTTP/1.1", f"Host: {url.netloc}", "Upgrade: websocket", "Connection: Upgrade",
                 f"Sec-WebSocket-Key: {key}", "Sec-WebSocket-Version: 13",
                 *(f"{name}: {value}" for name, value in headers.items())]
        self.socket.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.buffer = b""
        while b"\r\n\r\n" not in self.buffer:
            self.buffer += self.receive_bytes()
        head, self.buffer = self.buffer.split(b"\r\n\r\n", 1)
        status = head.split(b"\r\n", 1)[0].split()
        if len(status) < 2 or status[1] != b"101":
            self.socket.close()
            raise RuntimeError(f"T3 socket upgrade failed with HTTP {status[1].decode() if len(status) > 1 else '?'}")

    def receive_bytes(self) -> bytes:
        chunk = self.socket.recv(65536)
        if not chunk:
            raise RuntimeError("T3 socket closed")
        return chunk

    def read(self, size: int) -> bytes:
        while len(self.buffer) < size:
            self.buffer += self.receive_bytes()
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def send(self, payload: bytes, opcode: int = 1) -> None:
        size = len(payload)
        if size < 126:
            header = bytes([0x80 | opcode, 0x80 | size])
        elif size < 1 << 16:
            header = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack(">H", size)
        else:
            header = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack(">Q", size)
        mask = os.urandom(4)
        self.socket.sendall(header + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload)))

    def receive(self) -> str:
        message = b""
        while True:
            first, second = self.read(2)
            size = second & 0x7F
            if size == 126:
                size = struct.unpack(">H", self.read(2))[0]
            elif size == 127:
                size = struct.unpack(">Q", self.read(8))[0]
            payload = self.read(size)
            opcode = first & 0x0F
            if opcode == 8:
                raise RuntimeError("T3 socket closed")
            if opcode == 9:
                self.send(payload, 10)
            elif opcode in {0, 1}:
                message += payload
                if first & 0x80:
                    return message.decode()

    def close(self) -> None:
        self.socket.close()


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
        self.rpc = None
        self.request_id = 0
        return self

    def __exit__(self, *args):
        if self.rpc:
            self.rpc.close()
        result = subprocess.run([str(self.cli), "auth", "session", "revoke", "--base-dir", str(T3_HOME),
                                 self.session["sessionId"]],
                                capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError("Cannot revoke local T3 session")

    def request(self, path: str) -> dict:
        request = urllib.request.Request(self.origin + path, headers={
            "Authorization": "Bearer " + self.session["token"], PROTOCOL_HEADER: PROTOCOL_VERSION})
        try:
            with urllib.request.build_opener(RejectRedirects()).open(request, timeout=45) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"T3 request failed with HTTP {error.code}") from None

    def shell(self) -> dict:
        return self.request("/api/orchestration/shell")

    def dispatch(self, command: dict) -> dict:
        if self.rpc is None:
            self.rpc = WebSocket(self.origin, f"/ws?orchestrationProtocol={PROTOCOL_VERSION}",
                                 {"Authorization": "Bearer " + self.session["token"]})
        self.request_id += 1
        request_id = str(self.request_id)
        self.rpc.send(json.dumps({"_tag": "Request", "id": request_id, "tag": "orchestration.dispatchCommand",
                                  "payload": command, "headers": []}).encode())
        while True:
            messages = json.loads(self.rpc.receive())
            for message in messages if isinstance(messages, list) else [messages]:
                if message.get("_tag") != "Exit" or message.get("requestId") != request_id:
                    continue
                exit = message["exit"]
                if exit.get("_tag") == "Success":
                    return exit["value"]
                causes = exit.get("cause") or [{}]
                error = (causes[0].get("error") or {}).get("_tag") or causes[0].get("_tag", "unknown")
                raise RuntimeError(f"T3 rejected {command['type']}: {error}")


def create_worktree(target: Path) -> str:
    def git(*args):
        result = subprocess.run(["git", "-C", str(CHECKOUT), *args],
                                capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError("Release worktree operation failed")
        return result.stdout.strip()
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
