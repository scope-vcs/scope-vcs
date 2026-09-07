#!/usr/bin/env python3
"""Provision, observe, and remove only the disposable cache/database AWS stack."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

ACCOUNT = "957143340948"
REGION = "us-east-1"
ENVIRONMENT = "cache-db-staging-20260907"
STACK = f"scope-{ENVIRONMENT}"
ROLE = f"{STACK}-ci-admin"
CLUSTER = f"arn:aws:ecs:{REGION}:{ACCOUNT}:cluster/scope-vcs-{ENVIRONMENT}-runner"
LOG_GROUP = f"/scope-vcs/{ENVIRONMENT}/cloud-runner"
SECRET_PREFIX = f"scope-vcs/scope-vcs-{ENVIRONMENT}-runner/attempts/"
ROOT = Path(__file__).resolve().parents[2]


def aws(*args, timeout=180):
    result = subprocess.run(
        ["aws", *args, "--region", REGION, "--output", "json", "--no-cli-pager"],
        capture_output=True, text=True, timeout=timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"AWS {args[0]} {args[1]} failed: {result.stderr.strip()}")
    return json.loads(result.stdout) if result.stdout.strip() else {}


def save(folder, name, value):
    (folder / name).write_text(json.dumps(value, indent=2) + "\n")


def stack():
    try:
        value = aws("cloudformation", "describe-stacks", "--stack-name", STACK)["Stacks"][0]
    except RuntimeError as error:
        if f"Stack with id {STACK} does not exist" in str(error):
            return None
        raise
    assert value["StackName"] == STACK
    assert value["StackId"].startswith(f"arn:aws:cloudformation:{REGION}:{ACCOUNT}:stack/{STACK}/")
    return value


def outputs(value):
    result = {item["OutputKey"]: item["OutputValue"] for item in value.get("Outputs", [])}
    assert result["RunnerClusterArn"] == CLUSTER, "Unexpected experiment cluster"
    assert result["RunnerLogGroupName"] == LOG_GROUP, "Unexpected experiment log group"
    assert result["RailwayDispatcherUserName"] == f"{STACK}-railway-dispatcher"
    return result


def encrypt_dispatcher_key(folder, user):
    public_key = ROOT / "dev/cache-db-experiment/transfer-public.pem"
    # Check the transfer key before generating an AWS credential.
    subprocess.run(["openssl", "pkey", "-pubin", "-in", str(public_key), "-noout"], check=True)
    keys = aws("iam", "list-access-keys", "--user-name", user)["AccessKeyMetadata"]
    if keys:
        raise RuntimeError("Dispatcher already has an access key; refusing to create another")
    key = aws("iam", "create-access-key", "--user-name", user)["AccessKey"]
    plaintext = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", prefix="dispatcher-", delete=False) as file:
            plaintext = Path(file.name)
            os.chmod(plaintext, 0o600)
            json.dump({"AWS_ACCESS_KEY_ID": key["AccessKeyId"], "AWS_SECRET_ACCESS_KEY": key["SecretAccessKey"]}, file)
        subprocess.run([
            "openssl", "pkeyutl", "-encrypt", "-pubin", "-inkey", str(public_key),
            "-pkeyopt", "rsa_padding_mode:oaep", "-pkeyopt", "rsa_oaep_md:sha256",
            "-pkeyopt", "rsa_mgf1_md:sha256", "-in", str(plaintext),
            "-out", str(folder / "dispatcher-key.enc"),
        ], check=True, capture_output=True)
    except Exception:
        (folder / "dispatcher-key.enc").unlink(missing_ok=True)
        aws("iam", "delete-access-key", "--user-name", user, "--access-key-id", key["AccessKeyId"])
        raise
    finally:
        if plaintext is not None:
            plaintext.unlink(missing_ok=True)
    save(folder, "dispatcher.json", {"user": user, "encrypted": True})


def provision(folder, template):
    aws("cloudformation", "validate-template", "--template-body", f"file://{template}")
    existing = stack()
    if existing is None or existing["StackStatus"] == "REVIEW_IN_PROGRESS":
        change_name = f"{STACK}-{os.environ.get('GITHUB_RUN_ID', 'local')}-{int(time.time())}"
        change = aws(
            "cloudformation", "create-change-set", "--stack-name", STACK,
            "--change-set-name", change_name, "--change-set-type", "CREATE",
            "--template-body", f"file://{template}", "--capabilities", "CAPABILITY_NAMED_IAM",
            "--parameters", f"ParameterKey=Environment,ParameterValue={ENVIRONMENT}",
            "--tags", "Key=Project,Value=scope-vcs", f"Key=Environment,Value={ENVIRONMENT}",
        )
        save(folder, "change-set-created.json", change)
        aws("cloudformation", "wait", "change-set-create-complete", "--change-set-name", change["Id"], timeout=300)
        save(folder, "change-set.json", aws("cloudformation", "describe-change-set", "--change-set-name", change["Id"]))
        aws("cloudformation", "execute-change-set", "--change-set-name", change["Id"])
        aws("cloudformation", "wait", "stack-create-complete", "--stack-name", STACK, timeout=1200)
    elif existing["StackStatus"] != "CREATE_COMPLETE":
        raise RuntimeError(f"Refusing to change existing stack in {existing['StackStatus']}")
    current = stack()
    save(folder, "stack.json", current)
    config = outputs(current)
    save(folder, "outputs.json", config)
    encrypt_dispatcher_key(folder, config["RailwayDispatcherUserName"])


def describe_tasks():
    arns = set()
    for state in ["RUNNING", "STOPPED"]:
        arns.update(aws("ecs", "list-tasks", "--cluster", CLUSTER, "--desired-status", state)["taskArns"])
    tasks = []
    for start in range(0, len(arns), 100):
        batch = sorted(arns)[start:start + 100]
        result = aws("ecs", "describe-tasks", "--cluster", CLUSTER, "--tasks", *batch, "--include", "TAGS")
        if result.get("failures"):
            raise RuntimeError("Could not describe every experiment task")
        tasks.extend(result["tasks"])
    for task in tasks:
        assert task["clusterArn"] == CLUSTER
    return tasks


def redact_log(message):
    message = re.sub(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b", "[REDACTED_AWS_KEY]", message)
    message = re.sub(r"(?i)(authorization\s*[:=]\s*(?:bearer\s+)?)[^\s,]+", r"\1[REDACTED]", message)
    return message


def observe(folder, current):
    config = outputs(current)
    save(folder, "outputs.json", config)
    tasks = describe_tasks()
    # Overrides can contain runner environment variables. Archive only task metadata.
    allowed = ["taskArn", "clusterArn", "taskDefinitionArn", "lastStatus", "desiredStatus",
               "createdAt", "startedAt", "stoppedAt", "pullStartedAt", "pullStoppedAt",
               "stopCode", "stoppedReason", "cpu", "memory", "launchType", "platformVersion", "tags"]
    sanitized = []
    for task in tasks:
        row = {key: task[key] for key in allowed if key in task}
        row["containers"] = [{key: container[key] for key in ["name", "lastStatus", "exitCode", "imageDigest"] if key in container}
                             for container in task.get("containers", [])]
        sanitized.append(row)
    save(folder, "tasks.json", sanitized)
    logs = aws("logs", "filter-log-events", "--log-group-name", LOG_GROUP)
    for event in logs.get("events", []):
        event["message"] = redact_log(event["message"])
    save(folder, "logs.json", logs)
    return tasks, config


def cleanup(folder, current, retained_definitions):
    tasks, config = observe(folder, current)
    if any(task["lastStatus"] != "STOPPED" for task in tasks):
        raise RuntimeError("Refusing cleanup while experiment tasks are still active")
    # Check all definition ownership before deleting any resource.
    definitions = []
    # ECS stops listing old stopped tasks before a long experiment finishes.
    # Earlier observations retain their definitions; ownership is still checked here.
    definition_arns = {task["taskDefinitionArn"] for task in tasks} | set(retained_definitions)
    for arn in sorted(definition_arns):
        assert arn.startswith(f"arn:aws:ecs:{REGION}:{ACCOUNT}:task-definition/scope-runner-attempt_")
        definition = aws("ecs", "describe-task-definition", "--task-definition", arn)["taskDefinition"]
        assert definition["executionRoleArn"] == config["RunnerExecutionRoleArn"]
        assert all(container.get("logConfiguration", {}).get("options", {}).get("awslogs-group") == LOG_GROUP
                   for container in definition["containerDefinitions"])
        definitions.append((arn, definition["status"]))
    secrets = aws("secretsmanager", "list-secrets", "--filters", f"Key=name,Values={SECRET_PREFIX}")["SecretList"]
    secrets = [secret for secret in secrets if secret["Name"].startswith(SECRET_PREFIX)]
    # Recheck immediately before the first deletion, after potentially lengthy log archival.
    if any(task["lastStatus"] != "STOPPED" for task in describe_tasks()):
        raise RuntimeError("An experiment task started during cleanup; refusing deletion")
    deleted = {"taskDefinitions": [], "secrets": [], "dispatcherKeys": 0}
    save(folder, "cleanup.json", deleted)
    for arn, status in definitions:
        if status == "ACTIVE":
            aws("ecs", "deregister-task-definition", "--task-definition", arn)
        result = aws("ecs", "delete-task-definitions", "--task-definitions", arn)
        if result.get("failures"):
            raise RuntimeError("Failed to delete an experiment task definition")
        deleted["taskDefinitions"].append(arn)
        save(folder, "cleanup.json", deleted)
    for secret in secrets:
        aws("secretsmanager", "delete-secret", "--secret-id", secret["ARN"], "--force-delete-without-recovery")
        deleted["secrets"].append(secret["Name"])
        save(folder, "cleanup.json", deleted)
    user = config["RailwayDispatcherUserName"]
    for key in aws("iam", "list-access-keys", "--user-name", user)["AccessKeyMetadata"]:
        aws("iam", "delete-access-key", "--user-name", user, "--access-key-id", key["AccessKeyId"])
        deleted["dispatcherKeys"] += 1
    save(folder, "cleanup.json", deleted)
    aws("cloudformation", "delete-stack", "--stack-name", STACK)
    aws("cloudformation", "wait", "stack-delete-complete", "--stack-name", STACK, timeout=1200)
    deleted["stackDeleted"] = True
    save(folder, "cleanup.json", deleted)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--retained-task-definitions", type=Path,
                        default=ROOT / "dev/cache-db-experiment/retained-task-definitions.json")
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    operation = os.environ.get("REQUESTED_OPERATION") or json.loads((ROOT / "dev/cache-db-experiment/aws-request.json").read_text())["operation"]
    if operation not in {"provision", "observe", "cleanup"}:
        raise ValueError("Operation must be provision, observe, or cleanup")
    identity = aws("sts", "get-caller-identity")
    assert identity["Account"] == ACCOUNT
    assert identity["Arn"].startswith(f"arn:aws:sts::{ACCOUNT}:assumed-role/{ROLE}/"), "Use the experiment CI role"
    save(args.evidence, "operation.json", {"operation": operation, "identity": identity, "stack": STACK})
    try:
        if operation == "provision":
            provision(args.evidence, args.template.resolve())
        else:
            current = stack()
            if current is None:
                if operation == "cleanup":
                    save(args.evidence, "cleanup.json", {"stackAlreadyAbsent": True})
                else:
                    raise RuntimeError("Experiment stack does not exist")
            elif operation == "observe":
                observe(args.evidence, current)
            else:
                retained = (json.loads(args.retained_task_definitions.read_text())
                            if args.retained_task_definitions.exists() else [])
                assert isinstance(retained, list) and all(isinstance(arn, str) for arn in retained)
                cleanup(args.evidence, current, retained)
    except Exception as error:
        save(args.evidence, "error.json", {"error": str(error)})
        try:
            save(args.evidence, "stack-events.json", aws("cloudformation", "describe-stack-events", "--stack-name", STACK))
        except Exception:
            pass
        raise


if __name__ == "__main__":
    main()
