#!/usr/bin/env python3

import itertools
import json
import math
import os
import pathlib
import re
import shlex
import subprocess
import sys
import tomllib

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"
RUNNER_CAP = 20
SETUP_JOBS = {"ci.yml:plan", "scope-cli-build.yml:prepare"}
GATE_JOBS = {
    "ci.yml:required-pr-checks",
    "validate.yml:production-validation-gate",
    "validate-server.yml:server-validation-gate",
}
BUILTIN_PROFILES = {"dev": {}, "release": {}, "test": {"inherits": "dev"}, "bench": {"inherits": "release"}}
PROFILE_COMMANDS = {"build", "test", "run", "bench"}
NON_MAIN_EVENTS = [
    {"event_name": "pull_request", "ref": "refs/pull/1/merge"},
    {"event_name": "push", "ref": "refs/heads/scope/requests/req_1"},
    {"event_name": "workflow_dispatch", "ref": "refs/heads/feature"},
]
PULL_REQUEST = {
    "event_name": "pull_request",
    "ref": "refs/pull/1/merge",
    "repository": "scope-vcs/scope-vcs",
    "actor": "maintainer",
    "event": {"pull_request": {"head": {"repo": {"full_name": "scope-vcs/scope-vcs"}}}},
}
SCRIPT_REFERENCE = re.compile(r"(?:^|[\s;&|(])(?:\.\./|\./)*(dev/[\w./-]+)")
DOWNLOADED_KEY = re.compile(
    r"(?:curl|wget)\b[^\n]*(?:https?://\S+[/.](?:asc|gpg|key|pub|pem|crt)\b|/keyrings?/|gpg\s+--dearmor|apt-key)"
    r"|--recv-keys|\bkeyserver\b"
)

TOKEN = re.compile(
    r"\s*(?:(?P<string>'(?:[^']|'')*')|(?P<number>\d+(?:\.\d+)?)"
    r"|(?P<operator>==|!=|&&|\|\||[!(),])"
    r"|(?P<name>[A-Za-z_][\w-]*(?:\.[\w-]+)*))"
)


class Expression:
    def __init__(self, text, context):
        self.tokens = []
        position = 0
        text = text.strip()
        while position < len(text):
            match = TOKEN.match(text, position)
            if not match or match.end() == position:
                raise ValueError(f"cannot read expression: {text}")
            kind = match.lastgroup
            self.tokens.append((kind, match.group(kind)))
            position = match.end()
        self.index = 0
        self.context = context

    def value(self):
        result = self.either()
        if self.index != len(self.tokens):
            raise ValueError(f"unexpected {self.tokens[self.index][1]}")
        return result

    def peek(self):
        return self.tokens[self.index][1] if self.index < len(self.tokens) else None

    def take(self, expected=None):
        kind, text = self.tokens[self.index]
        if expected is not None and text != expected:
            raise ValueError(f"expected {expected}, found {text}")
        self.index += 1
        return kind, text

    def either(self):
        left = self.both()
        while self.peek() == "||":
            self.take()
            right = self.both()
            left = left if truthy(left) else right
        return left

    def both(self):
        left = self.comparison()
        while self.peek() == "&&":
            self.take()
            right = self.comparison()
            left = right if truthy(left) else left
        return left

    def comparison(self):
        left = self.unary()
        while self.peek() in ("==", "!="):
            operator = self.take()[1]
            left = equal(left, self.unary()) == (operator == "==")
        return left

    def unary(self):
        if self.peek() == "!":
            self.take()
            return not truthy(self.unary())
        return self.primary()

    def primary(self):
        kind, text = self.take()
        if text == "(":
            result = self.either()
            self.take(")")
            return result
        if kind == "string":
            return text[1:-1].replace("''", "'")
        if kind == "number":
            return float(text)
        if text in ("true", "false"):
            return text == "true"
        if text == "null":
            return None
        if self.peek() == "(":
            self.take()
            arguments = []
            while self.peek() != ")":
                arguments.append(self.either())
                if self.peek() == ",":
                    self.take()
            self.take(")")
            return call(text, arguments, self.context)
        return lookup(self.context, text)


def truthy(value):
    return value not in (None, False, "", 0) and not (isinstance(value, float) and math.isnan(value))


def number(value):
    if value is None:
        return 0.0
    if isinstance(value, (bool, int, float)):
        return float(value)
    if not isinstance(value, str):
        return math.nan
    try:
        parsed = json.loads(value, parse_constant=lambda _: math.nan) if value.strip() else 0
        return float(parsed) if type(parsed) in (int, float) else math.nan
    except ValueError:
        return math.nan


def equal(left, right):
    if isinstance(left, str) and isinstance(right, str):
        return left.lower() == right.lower()
    if left is None and right is None:
        return True
    return number(left) == number(right)


def text_of(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    if isinstance(value, (dict, list)):
        return json.dumps(value)
    return "" if value is None else str(value)


def call(name, arguments, context):
    needs = context.get("needs", {})
    if name == "always":
        return True
    if name in ("cancelled", "failure"):
        return False
    if name == "success":
        return all(need.get("result") == "success" for need in needs.values())
    if name == "startsWith":
        return text_of(arguments[0]).lower().startswith(text_of(arguments[1]).lower())
    if name == "contains":
        haystack = arguments[0]
        if isinstance(haystack, list):
            return any(equal(item, arguments[1]) for item in haystack)
        return text_of(arguments[1]).lower() in text_of(haystack).lower()
    if name == "fromJSON":
        return json.loads(arguments[0]) if isinstance(arguments[0], str) else arguments[0]
    raise ValueError(f"unsupported expression function {name}()")


def lookup(context, path):
    value = context
    for part in path.split("."):
        if not isinstance(value, dict):
            return None
        value = value.get(part)
    return value


def evaluate(value, context):
    if not isinstance(value, str):
        return value
    whole = re.fullmatch(r"\s*\$\{\{(.*?)\}\}\s*", value, re.DOTALL)
    if whole:
        return Expression(whole.group(1), context).value()
    if "${{" not in value:
        return value
    return re.sub(r"\$\{\{(.*?)\}\}", lambda match: text_of(Expression(match.group(1), context).value()), value,
                  flags=re.DOTALL)


def condition(value, context):
    if value is None:
        return call("success", [], context)
    if isinstance(value, str) and "${{" not in value:
        return truthy(Expression(value, context).value())
    return truthy(evaluate(value, context))


def load(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


def needs_of(job):
    needs = job.get("needs") or []
    return [needs] if isinstance(needs, str) else list(needs)


def ordered(jobs):
    done, order = set(), []
    while len(order) < len(jobs):
        ready = [name for name, job in jobs.items() if name not in done and set(needs_of(job)) <= done]
        if not ready:
            raise ValueError("job needs form a cycle")
        for name in ready:
            done.add(name)
            order.append(name)
    return order


def workflow_inputs(document, supplied, context):
    declared = ((document.get(True) or document.get("on") or {}).get("workflow_call") or {}).get("inputs") or {}
    inputs = {}
    for name, spec in declared.items():
        value = evaluate(supplied[name], context) if name in supplied else spec.get("default")
        if spec.get("type") == "boolean" and isinstance(value, str):
            value = value.strip().lower() == "true"
        inputs[name] = value
    return inputs


def matrix_entries(matrix):
    if not matrix:
        return [{}]
    if set(matrix) - {"include"} and "include" in matrix or "exclude" in matrix:
        raise ValueError("matrices that combine axes with include or exclude are not modeled")
    if "include" in matrix:
        return list(matrix["include"])
    axes = sorted(matrix)
    return [dict(zip(axes, values)) for values in itertools.product(*(matrix[axis] for axis in axes))]


def job_outputs(job_id, inputs):
    if job_id == "ci.yml:plan":
        return {name: "true" for name in load("ci.yml")["jobs"]["plan"]["outputs"]}
    if job_id == "scope-cli-build.yml:prepare":
        selected = subprocess.run(
            ["node", ".github/scripts/select-cli-distribution-targets.mjs", "--mode", inputs["distribution_mode"]],
            cwd=ROOT, check=True, capture_output=True, text=True,
        )
        return {"matrix": selected.stdout.strip()}
    return {}


def simulate(workflow, github, supplied=None, caller=None, waits_for=frozenset(), instance=""):
    document = load(workflow)
    inputs = workflow_inputs(document, supplied or {}, caller or {})
    jobs, results, nodes = document["jobs"], {}, []
    for name in ordered(jobs):
        job, job_id = jobs[name], f"{workflow}:{name}"
        needs = needs_of(job)
        context = {"github": github, "inputs": inputs,
                   "needs": {need: {"result": results[need]["result"], "outputs": results[need]["outputs"]}
                             for need in needs}}
        if not condition(job.get("if"), context):
            results[name] = {"result": "skipped", "outputs": {}, "nodes": set()}
            continue
        before = set(waits_for).union(*(results[need]["nodes"] for need in needs))
        matrix = evaluate((job.get("strategy") or {}).get("matrix"), context)
        created = set()
        for entry in matrix_entries(matrix):
            label = text_of(entry.get("label") or entry.get("group")) or ", ".join(map(text_of, entry.values()))
            if "uses" in job:
                child = simulate(job["uses"].rsplit("/", 1)[1], github, job.get("with") or {},
                                 {**context, "matrix": entry}, frozenset(before),
                                 f"{instance} [{label}]" if label else instance)
                nodes.extend(child)
                created.update(node["id"] for node in child)
                continue
            node_id = f"{job_id} ({label}){instance}" if label else f"{job_id}{instance}"
            nodes.append({"id": node_id, "job": job_id, "definition": job, "waits_for": before,
                          "workflow_env": document.get("env") or {},
                          "context": {**context, "matrix": entry}})
            created.add(node_id)
        outputs = {} if "uses" in job else job_outputs(job_id, inputs)
        results[name] = {"result": "success", "outputs": outputs, "nodes": created}
    return nodes


def full_pull_request_run():
    nodes = simulate("ci.yml", PULL_REQUEST)
    by_id = {node["id"]: node for node in nodes}
    for node in nodes:
        pending, closure = list(node["waits_for"]), set()
        while pending:
            current = pending.pop()
            if current not in closure:
                closure.add(current)
                pending.extend(by_id[current]["waits_for"])
        node["after"] = closure
    return nodes


def peak_concurrency(nodes):
    ids = [node["id"] for node in nodes]
    later = {node["id"]: [other["id"] for other in nodes if node["id"] in other["after"]] for node in nodes}
    match = {}

    def augment(start, seen):
        for target in later[start]:
            if target not in seen:
                seen.add(target)
                if target not in match or augment(match[target], seen):
                    match[target] = start
                    return True
        return False

    return len(ids) - sum(augment(node_id, set()) for node_id in ids)


def runner_cap_violations(nodes):
    peak = peak_concurrency(nodes)
    if peak <= RUNNER_CAP:
        return []
    return [f"a full pull request run can have {peak} jobs running at once; GitHub's free plan runs {RUNNER_CAP}"]


def serial_wait_violations(nodes):
    job_of = {node["id"]: node["job"] for node in nodes}
    violations = []
    for node in nodes:
        if node["job"] in GATE_JOBS:
            continue
        jobs = {job_of[node_id] for node_id in node["after"]}
        violations.extend(f"{node['id']} waits for {job}" for job in sorted(jobs - SETUP_JOBS))
    return violations


def workspace_profiles(manifest):
    directory = (ROOT / manifest).resolve().parent
    while True:
        document = tomllib.loads((directory / "Cargo.toml").read_text())
        if "workspace" in document or directory == ROOT:
            return directory.relative_to(ROOT), document.get("profile", {})
        directory = directory.parent


def configured_profiles(workspace, env):
    profiles = {}
    for config in (ROOT / ".cargo/config.toml", ROOT / workspace / ".cargo/config.toml"):
        if config.is_file():
            for name, table in tomllib.loads(config.read_text()).get("profile", {}).items():
                profiles.setdefault(name, {}).update(table)
    for key, value in env.items():
        override = re.fullmatch(r"CARGO_PROFILE_([A-Z0-9_]+)_(LTO|CODEGEN_UNITS)", key)
        if not override:
            continue
        text = text_of(value).strip().lower()
        name = override.group(1).lower().replace("_", "-")
        if override.group(2) == "LTO":
            profiles.setdefault(name, {})["lto"] = text == "true" if text in ("true", "false") else text
        else:
            profiles.setdefault(name, {})["codegen-units"] = int(text) if text.isdigit() else text
    return profiles


def profile_settings(profiles, name):
    settings = {}
    while name:
        declared = {**BUILTIN_PROFILES.get(name, {}), **profiles.get(name, {})}
        settings = {**declared, **settings}
        name = declared.get("inherits") if declared.get("inherits") != name else None
    return settings


def cargo_commands(text):
    for line in re.split(r"\n|&&|\|\||;", text):
        if "deployment-components.mjs build" in line:
            yield "Cargo.toml", "release"
            continue
        try:
            words = shlex.split(line)
        except ValueError:
            words = line.split()
        for index, word in enumerate(words):
            if word in ("cargo", "cross") and index + 1 < len(words) and words[index + 1] in PROFILE_COMMANDS:
                arguments = [part for word in words[index + 2:] for part in (
                    word.split("=", 1) if word.startswith(("--profile=", "--manifest-path=")) else [word]
                )]
                profile = "release" if "--release" in arguments else (
                    words[index + 1] if words[index + 1] in ("test", "bench") else "dev")
                manifest = None
                for flag, value in zip(arguments, arguments[1:]):
                    if flag == "--profile":
                        profile = value
                    if flag == "--manifest-path":
                        manifest = value
                yield manifest, profile
                break


def step_cargo_commands(text, directory, through=None, seen=frozenset()):
    for manifest, profile in cargo_commands(text):
        yield str(pathlib.Path(directory) / (manifest or "Cargo.toml")), profile, through
    for match in SCRIPT_REFERENCE.finditer(text):
        script = ROOT / match.group(1)
        if script.suffix in ("", ".sh") and script.is_file() and os.access(script, os.X_OK) and script not in seen:
            yield from step_cargo_commands(script.read_text(), ".", through or match.group(1), seen | {script})


def lto_violations(nodes):
    violations = []
    for node in nodes:
        job = node["definition"]
        default_directory = ((job.get("defaults") or {}).get("run") or {}).get("working-directory", ".")
        for step in job.get("steps") or []:
            if "run" not in step:
                continue
            text = evaluate(step["run"], node["context"])
            directory = evaluate(step.get("working-directory", default_directory), node["context"])
            env = {name: evaluate(value, node["context"])
                   for name, value in {**node["workflow_env"], **(job.get("env") or {}), **(step.get("env") or {})}.items()}
            for manifest, profile, through in step_cargo_commands(text, directory):
                workspace, declared = workspace_profiles(manifest)
                overrides = configured_profiles(workspace, env)
                profiles = {name: {**declared.get(name, {}), **overrides.get(name, {})}
                            for name in {*declared, *overrides}}
                settings = profile_settings(profiles, profile)
                if settings.get("lto") in (True, "fat") or settings.get("codegen-units") == 1:
                    violations.append(
                        f"{node['id']} step '{step.get('name', step['run'][:40])}' builds {workspace}/Cargo.toml"
                        f"{f' through {through}' if through else ''} "
                        f"with profile {profile} (lto={settings.get('lto')}, "
                        f"codegen-units={settings.get('codegen-units')})"
                    )
    return violations


def api_build_violations(nodes):
    violations = []
    for node in nodes:
        if node["job"] != "scope-api-ci.yml:deployment-binaries":
            continue
        components = text_of(node["context"]["matrix"].get("components")).split()
        if "api" in components and components != ["api"]:
            violations.append(f"{node['id']} builds api together with {' '.join(c for c in components if c != 'api')}")
    return violations


def workflow_steps():
    for path in sorted([*WORKFLOWS.glob("*.yml"), *(ROOT / ".github/actions").glob("*/action.yml")]):
        document = yaml.safe_load(path.read_text()) or {}
        jobs = [(f"{path.name}:{name}", job) for name, job in (document.get("jobs") or {}).items()]
        if "runs" in document:
            jobs.append((None, document["runs"]))
        for name, job in jobs:
            for step in job.get("steps") or []:
                yield path.name if path.parent == WORKFLOWS else f"{path.parent.name}/action.yml", name, job, step


def cache_violations(nodes):
    simulated = {}
    for node in nodes:
        simulated.setdefault(node["job"], []).append(node["context"]["matrix"])
    violations = []
    for path, job_id, job, step in workflow_steps():
        if not str(step.get("uses", "")).startswith("Swatinem/rust-cache@"):
            continue
        save_if = (step.get("with") or {}).get("save-if", True)
        matrix = (job.get("strategy") or {}).get("matrix")
        entries = simulated.get(job_id) or (matrix_entries(matrix) if isinstance(matrix, dict) else [{}])
        for github in NON_MAIN_EVENTS:
            contexts = ({"github": github, "inputs": {}, "matrix": entry, "needs": {}} for entry in entries)
            if any(truthy(evaluate(save_if, context)) for context in contexts):
                violations.append(
                    f"{path} step '{step.get('name', 'rust-cache')}' saves its cache on {github['event_name']} "
                    f"to {github['ref']}"
                )
    return violations


def tracked_text_files():
    listed = subprocess.run(["git", "ls-files", ".github", "dev", "deploy"], cwd=ROOT, check=True,
                            capture_output=True, text=True).stdout.split()
    for name in listed:
        path = ROOT / name
        test_source = name.endswith(".test.mjs") or path.name.startswith("test-") or "fixtures" in name
        if path.is_file() and path.suffix in ("", ".sh", ".yml", ".yaml", ".mjs", ".js", ".py") \
                and not test_source and path != pathlib.Path(__file__).resolve():
            try:
                yield name, path.read_text()
            except UnicodeDecodeError:
                continue


def downloaded_key_violations(files):
    return [f"{name} downloads a signing key; commit it next to the script that pins it"
            for name, text in files if DOWNLOADED_KEY.search(text)]


def main():
    nodes = full_pull_request_run()
    violations = [
        *runner_cap_violations(nodes),
        *serial_wait_violations(nodes),
        *lto_violations(nodes),
        *api_build_violations(nodes),
        *cache_violations(nodes),
        *downloaded_key_violations(tracked_text_files()),
    ]
    if violations:
        print("CI critical path regressions:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1
    print(f"CI critical path: a full pull request run starts {len(nodes)} jobs, "
          f"at most {peak_concurrency(nodes)} at once.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
