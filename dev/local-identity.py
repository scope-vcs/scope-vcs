#!/usr/bin/env python3
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import urlopen

spec = importlib.util.spec_from_file_location("local_process", Path(__file__).with_name("local-process.py"))
local_process = importlib.util.module_from_spec(spec)
spec.loader.exec_module(local_process)

BUILD_FILE = "build.json"
SOURCE_FILE = "build-source.json"
SERVICES = ("api", "worker", "media", "media-worker", "web")
PROBES = {"api": "/readyz", "media": "/healthz", "media-worker": "/readyz", "web": "/"}
PRIVATE_PROBES = {"web": "/", "api": "/readyz", "media": "/healthz"}


def git(root, *args, env=None):
    result = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, env=env)
    return result.stdout.strip() if result.returncode == 0 else None


def source_identity(root):
    head = git(root, "rev-parse", "--verify", "HEAD^{commit}")
    index = git(root, "rev-parse", "--path-format=absolute", "--git-path", "index")
    if head is None or index is None:
        return None
    with tempfile.TemporaryDirectory(prefix="scope-dev-index-") as temp:
        scratch = Path(temp) / "index"
        if Path(index).is_file():
            shutil.copy2(index, scratch)
        env = {**os.environ, "GIT_INDEX_FILE": str(scratch)}
        if git(root, "add", "--all", env=env) is None:
            return None
        tree = git(root, "write-tree", env=env)
    if tree is None:
        return None
    return {"head": head, "tree": tree, "uncommitted": tree != git(root, "rev-parse", "HEAD^{tree}")}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def build_start(root, state):
    state.mkdir(parents=True, exist_ok=True)
    (state / BUILD_FILE).unlink(missing_ok=True)
    write_json(state / SOURCE_FILE, {"source": source_identity(root)})


def build_finish(root, state, entries):
    before = (read_json(state / SOURCE_FILE) or {}).get("source")
    after = source_identity(root)
    record = {"checkout": str(root), "built_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
              "source": before, "unknown": None, "services": {}}
    if before is None or after is None:
        record["source"] = None
        record["unknown"] = "git could not identify the checkout when up built it"
    elif before["tree"] != after["tree"]:
        record["source"] = None
        record["unknown"] = "the checkout changed while up was building"
    for entry in entries:
        name, _, binary = entry.partition("=")
        if binary:
            stat = os.stat(binary)
            record["services"][name] = {"binary": binary, "device": stat.st_dev, "inode": stat.st_ino}
        else:
            record["services"][name] = {"live": str(root / name)}
    write_json(state / BUILD_FILE, record)
    (state / SOURCE_FILE).unlink(missing_ok=True)


def forget_build(state):
    (state / BUILD_FILE).unlink(missing_ok=True)
    (state / SOURCE_FILE).unlink(missing_ok=True)


def listener(port):
    result = subprocess.run(["ss", "-H", "-ltnp", f"sport = :{port}"], capture_output=True, text=True)
    for token in result.stdout.split("pid=")[1:]:
        digits = token.split(",", 1)[0]
        if digits.isdigit():
            return int(digits)
    return None


def proc_link(pid, name):
    try:
        return os.readlink(f"/proc/{pid}/{name}")
    except OSError:
        return "unknown"


def proc_environment(pid, names):
    try:
        entries = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
    except OSError:
        return None
    found = {}
    for entry in entries:
        name, _, value = entry.decode(errors="replace").partition("=")
        if name in names:
            found[name] = value
    return found


def auth_mode(service, pid):
    if service == "api":
        env = proc_environment(pid, {"CLERK_ISSUER"})
        if env is None:
            return "unknown: cannot read the process environment"
        host = urlsplit(env.get("CLERK_ISSUER", "")).hostname
        return f"Clerk tokens from {host}" if host else "unknown: no Clerk issuer"
    if service == "web":
        env = proc_environment(pid, {"VITE_SCOPE_WEB_FORCE_SIGNED_OUT"})
        if env is None:
            return "unknown: cannot read the process environment"
        mode = env.get("VITE_SCOPE_WEB_FORCE_SIGNED_OUT")
        return {"1": "signed out (anonymous viewer)", "0": "Clerk sign-in"}.get(
            mode, "unknown: the sign-in mode is not set")
    return "no viewer sign-in"


def actor(api_pid, web_auth):
    if web_auth.startswith("signed out"):
        return "anonymous viewer"
    if web_auth != "Clerk sign-in" or api_pid is None:
        return "unknown"
    email = (proc_environment(api_pid, {"SCOPE_DEV_USER_EMAIL"}) or {}).get("SCOPE_DEV_USER_EMAIL")
    return f"sign in as {email}, the seed user who owns the seeded repositories" if email else "unknown"


def http_status(url):
    try:
        with urlopen(url, timeout=3) as response:
            return f"HTTP {response.status}"
    except HTTPError as error:
        return f"HTTP {error.code}"
    except (OSError, ValueError):
        return "unreachable"


class Service:
    def __init__(self, name, port, state, build):
        self.name = name
        self.port = port
        self.owner = local_process.owned_identity(state / f"{name}.pid")
        self.recorded = (state / f"{name}.pid").exists()
        self.expected = (build or {}).get("services", {}).get(name)
        self.pid = listener(port)
        identity = local_process.process_identity(self.pid) if self.pid else None
        self.owned = bool(self.owner and identity and identity["session"] == self.owner["session"])
        self.problem = self.find_problem()

    def find_problem(self):
        if not self.owner:
            return "stale launcher record" if self.recorded else "not running"
        if self.pid is None:
            return f"nothing listens on port {self.port}"
        if not self.owned:
            return f"port {self.port} is served by another process"
        if self.expected is None:
            return "up did not record what this process runs"
        if "live" in self.expected:
            if proc_link(self.pid, "cwd") != self.expected["live"]:
                return f"serves {proc_link(self.pid, 'cwd')}, not {self.expected['live']}"
            return None
        try:
            stat = os.stat(f"/proc/{self.pid}/exe")
        except OSError:
            return "cannot read the running binary"
        if (stat.st_dev, stat.st_ino) != (self.expected["device"], self.expected["inode"]):
            return "the running binary is not the one up built"
        return None

    def describe(self, build):
        if self.pid and not self.owned:
            exe = proc_link(self.pid, "exe")
            return (f"{self.label()}: not this stack, port {self.port} is served by pid {self.pid} "
                    f"from {proc_link(self.pid, 'cwd')} running {exe}")
        if self.problem:
            return f"{self.label()}: {self.problem}"
        if "live" in self.expected:
            revision = f"serves {self.expected['live']} live from the working tree"
        else:
            revision = f"runs {build_revision(build)}"
        probe = f"; {http_status(f'http://localhost:{self.port}{PROBES[self.name]}')}" if self.name in PROBES else ""
        return (f"{self.label()}: pid {self.pid} on port {self.port}; {revision}; "
                f"auth {auth_mode(self.name, self.pid)}{probe}")

    def label(self):
        return self.name.replace("-", " ")


def build_revision(build):
    if build is None:
        return "unknown: up has not recorded a build"
    if build["source"] is None:
        return f"unknown: {build['unknown']}"
    source = build["source"]
    return source["head"][:12] + (" with uncommitted changes" if source["uncommitted"] else "")


def relation(root, build, current):
    if build is None or build["source"] is None or current is None:
        return "unknown"
    built = build["source"]
    if built["tree"] == current["tree"]:
        return "the running build matches the checkout"
    if built["head"] == current["head"]:
        return "the uncommitted changes differ from the running build"
    if git(root, "merge-base", "--is-ancestor", built["head"], current["head"]) is not None:
        ahead = git(root, "rev-list", "--count", f"{built['head']}..{current['head']}")
        return f"HEAD is {ahead} commits ahead of the running build"
    return "the running build is not an ancestor of HEAD"


def private_target(origin):
    parts = urlsplit(origin)
    if parts.scheme != "https":
        return None
    try:
        serve = json.loads(subprocess.run(["tailscale", "serve", "status", "--json"], capture_output=True,
                                          text=True, timeout=5).stdout or "{}")
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None
    host = f"{parts.hostname}:{parts.port or 443}"
    proxy = ((serve.get("Web") or {}).get(host, {}).get("Handlers") or {}).get("/", {}).get("Proxy", "")
    target = urlsplit(proxy)
    return target.port if target.hostname in ("127.0.0.1", "localhost") else None


def private_checks(privates, ports):
    checks = []
    for name, origin in privates.items():
        if not origin:
            checks.append((f"private {name}: unavailable, no tailnet origin for this stack", None))
            continue
        target = private_target(origin)
        if target != ports[name]:
            reached = f"port {target}" if target else "an unknown target"
            problem = f"{origin} reaches {reached}, not this stack's port {ports[name]}"
            checks.append((f"private {name}: {problem}", f"private {name}: {problem}"))
        else:
            probe = http_status(origin + PRIVATE_PROBES[name])
            if probe == "unreachable":
                problem = f"{origin} proxies to this stack's port {ports[name]} but is unreachable"
                checks.append((f"private {name}: {problem}", f"private {name}: {problem}"))
            else:
                checks.append((f"private {name}: {origin} ({probe})", None))
    return checks


def report(args):
    root, state = Path(args.root).resolve(), Path(args.state)
    build = read_json(state / BUILD_FILE)
    current = source_identity(root)
    services = {name: Service(name, args.port[name], state, build) for name in SERVICES}
    lines = [f"checkout: {root}"]
    if current is None:
        lines.append("checkout HEAD: unknown")
    else:
        dirty = ", with uncommitted changes" if current["uncommitted"] else ""
        lines.append(f"checkout HEAD: {current['head'][:12]}{dirty}")
    lines.append(f"running build: {build_revision(build)}"
                 + (f", built {build['built_at']}" if build and build["source"] else ""))
    lines.append(f"build vs checkout: {relation(root, build, current)}")
    lines.extend(service.describe(build) for service in services.values())
    return root, build, current, services, lines


def status(args):
    _, _, _, services, lines = report(args)
    lines.extend(line for line, _ in private_checks(args.private, args.port))
    web, api = services["web"], services["api"]
    if not web.problem and not api.problem:
        lines.append(f"actor: {actor(api.pid, auth_mode('web', web.pid))}")
    print("\n".join(lines))


def change_failures(root, build, current, change):
    if build is None:
        return ["up has not recorded a build for this checkout"]
    if build["source"] is None:
        return [f"the running revision is unknown: {build['unknown']}"]
    if current is None:
        return ["git cannot identify the checkout"]
    if change is None:
        if build["source"]["tree"] != current["tree"]:
            return [f"the stack does not serve the checkout: {relation(root, build, current)}"]
        return []
    commit = git(root, "rev-parse", "--verify", f"{change}^{{commit}}")
    if commit is None:
        return [f"{change} is not a commit in this checkout"]
    identities = (("running build", build["source"]), ("checkout", current))
    return [f"the {label} has uncommitted changes, so it cannot be shown to contain {commit[:12]}"
            if identity["uncommitted"] else f"the {label} at {identity['head'][:12]} does not contain {commit[:12]}"
            for label, identity in identities
            if identity["uncommitted"] or git(root, "merge-base", "--is-ancestor", commit, identity["head"]) is None]


def verify(args):
    root, build, current, services, lines = report(args)
    checks = private_checks(args.private, args.port)
    lines.extend(line for line, _ in checks)
    failures = change_failures(root, build, current, args.change)
    failures.extend(f"{services[name].label()}: {services[name].problem}"
                    for name in (build or {}).get("services", {}) if services[name].problem)
    failures.extend(failure for _, failure in checks if failure)
    print("\n".join(lines))
    if failures:
        print("not verified: results from this stack do not prove the change")
        print("\n".join(f"  {failure}" for failure in failures))
        sys.exit(1)
    web_auth = auth_mode("web", services["web"].pid) if "web" in build["services"] else "web not served"
    print(f"verified: {build_revision(build)} from {root}" + (f", contains {args.change}" if args.change else ""))
    print(f"auth: {web_auth}; actor: {actor(services['api'].pid, web_auth)}")


def tailnet_host():
    try:
        result = subprocess.run(["tailscale", "status", "--json"], capture_output=True,
                                text=True, timeout=5)
        node = json.loads(result.stdout)
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return
    if node.get("BackendState") == "Running" and (node.get("Self") or {}).get("DNSName"):
        print(node["Self"]["DNSName"].rstrip("."))


def pairs(values):
    found = {}
    for value in values or []:
        name, _, setting = value.partition("=")
        found[name] = setting
    return found


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("build-start", "build-finish", "forget-build"):
        command = commands.add_parser(name)
        command.add_argument("root")
        command.add_argument("state")
        if name == "build-finish":
            command.add_argument("services", nargs="+")
    for name in ("status", "verify"):
        command = commands.add_parser(name)
        command.add_argument("--root", required=True)
        command.add_argument("--state", required=True)
        command.add_argument("--port", action="append", required=True)
        command.add_argument("--private", action="append")
        if name == "verify":
            command.add_argument("change", nargs="?")
    commands.add_parser("tailnet-host")
    args = parser.parse_args()
    if args.command == "tailnet-host":
        tailnet_host()
        return
    if args.command in ("status", "verify"):
        args.port = {name: int(port) for name, port in pairs(args.port).items()}
        args.private = pairs(args.private)
        (status if args.command == "status" else verify)(args)
        return
    root, state = Path(args.root).resolve(), Path(args.state)
    if args.command == "build-start":
        build_start(root, state)
    elif args.command == "build-finish":
        build_finish(root, state, args.services)
    else:
        forget_build(state)


if __name__ == "__main__":
    main()
