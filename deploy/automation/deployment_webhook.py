import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import urllib.parse
import urllib.request
import uuid

from deployment_diagnostics import diagnostic, operation
from deployment_policy import trusted_run
from deployment_runtime import REPOSITORY, RejectRedirects, github


def source_event(event, name):
    if name != "workflow_dispatch":
        return event
    run_id = event.get("inputs", {}).get("run_id", "")
    if not isinstance(run_id, str) or not re.fullmatch(r"[1-9][0-9]*", run_id):
        raise ValueError("Probe must name an existing main Release run")
    with operation("github.webhook-probe"):
        run = github(f"actions/runs/{run_id}")
    return {"action": "completed" if run["status"] == "completed" else "in_progress",
            "repository": {"full_name": REPOSITORY}, "workflow_run": run}


def forward(event, url, secret):
    run = event.get("workflow_run", {})
    if (event.get("action") not in {"requested", "in_progress", "completed"}
            or event.get("repository", {}).get("full_name") != REPOSITORY
            or not trusted_run(run)):
        return {"ignored": True}
    with operation("github.webhook-source"):
        current = github(f"actions/runs/{int(run['id'])}")
    if not trusted_run(current) or current["id"] != run["id"] or current["run_attempt"] < run["run_attempt"]:
        return {"ignored": True}
    parsed = urllib.parse.urlparse(url)
    if (parsed.scheme != "https" or parsed.hostname != "relay.t3.codes" or parsed.username or parsed.password
            or parsed.port not in {None, 443} or not parsed.path.startswith("/v1/hooks/") or not secret):
        raise ValueError("Configure the T3 Connect URL and shared signing secret")
    delivery = str(uuid.uuid5(uuid.NAMESPACE_URL, f"{REPOSITORY}/{run['id']}/{run['run_attempt']}/{event['action']}"))
    payload = {"event": "workflow_run", "action": event["action"], "repository": REPOSITORY,
               "run_id": run["id"], "attempt": run["run_attempt"], "delivery": delivery}
    body = json.dumps(payload, separators=(",", ":")).encode()
    signature = "sha256=" + hmac.new(secret.encode(), body, hashlib.sha256).hexdigest()
    request = urllib.request.Request(url, data=body, headers={
        "Content-Type": "application/json", "User-Agent": "ScopeDeploymentSupervisor/1.0",
        "X-GitHub-Event": "workflow_run",
        "X-GitHub-Delivery": delivery, "X-Hub-Signature-256": signature}, method="POST")
    with operation("t3.signed-webhook-delivery"):
        with urllib.request.build_opener(RejectRedirects()).open(request, timeout=45) as response:
            if response.status not in {200, 202}:
                raise RuntimeError("T3 webhook was not accepted")
    return {"sent": delivery, "run_id": run["id"], "attempt": run["run_attempt"]}


if __name__ == "__main__":
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        event = source_event(event, os.environ["GITHUB_EVENT_NAME"])
        print(json.dumps(forward(event, os.environ.get("SCOPE_DEPLOYMENT_WEBHOOK_URL", ""),
                                 os.environ.get("SCOPE_DEPLOYMENT_WEBHOOK_SECRET", ""))), flush=True)
    except Exception as error:
        print(json.dumps({"failed": diagnostic(error)}), flush=True)
        raise SystemExit(1)
