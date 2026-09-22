"""The broker constructs every privileged AWS input from trusted configuration."""

import re

from journal import error_code
from protocol import Pending


class ProviderRejected(Exception):
    def __init__(self, reason, request_id=None):
        super().__init__(reason)
        self.reason = reason
        self.request_id = request_id


def request_id(response):
    metadata = response.get("ResponseMetadata") if isinstance(response, dict) else None
    value = metadata.get("RequestId") if isinstance(metadata, dict) else None
    return value if isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9-]{1,128}", value) else None


def rejection_reason(reason, detail=""):
    text = f"{reason} {detail}".lower()
    if "quota" in text or "limit exceeded" in text or "limit on the number of tasks" in text or "limit on the number of vcpus" in text:
        return "quota"
    if reason.upper() in {"RESOURCE:CPU", "RESOURCE:MEMORY", "RESOURCE:ENI", "RESOURCE:PORTS"} or "capacity is unavailable" in text or "insufficient capacity" in text:
        return "capacity"
    return "permanent"


def tags(attempt):
    return [
        {"key": "Project", "value": "scope-vcs"},
        {"key": "Component", "value": "cloud-runner"},
        {"key": "AttemptId", "value": attempt},
    ]


class Provider:
    def __init__(self, ecs, secrets, settings):
        self.ecs = ecs
        self.secrets = secrets
        self.settings = settings

    def secret_name(self, attempt):
        cluster = self.settings.cluster.rsplit("/", 1)[-1]
        return f"scope-vcs/{cluster}/attempts/{attempt}"

    def prepare(self, record, token):
        attempt = record["attempt_id"]
        name = self.secret_name(attempt)
        try:
            result = self.secrets.create_secret(
                Name=name, ClientRequestToken=attempt, SecretString=token,
                Tags=[{"Key": tag["key"], "Value": tag["value"]} for tag in tags(attempt)],
            )
        except Exception as error:
            if error_code(error) != "ResourceExistsException":
                raise
            # Only the broker owns this deterministic name. The durable token
            # hash is checked before this call; an existing value is never replaced.
            result = self.secrets.describe_secret(SecretId=name)
        container = {
            "name": "scope-runner", "image": record["image"], "essential": True,
            "entryPoint": ["/scope/bin/scope-runner-runtime"],
            "secrets": [{"name": "SCOPE_BOOTSTRAP_TOKEN", "valueFrom": result["ARN"]}],
            "logConfiguration": {
                "logDriver": "awslogs",
                "options": {"awslogs-group": self.settings.log_group, "awslogs-region": self.settings.region, "awslogs-stream-prefix": "runner"},
            },
        }
        if self.settings.registry_secret and registry_host(record["image"]) == self.settings.registry_host:
            container["repositoryCredentials"] = {"credentialsParameter": self.settings.registry_secret}
        definition = self.ecs.register_task_definition(
            family="scope-runner-" + attempt,
            networkMode="awsvpc", requiresCompatibilities=["FARGATE"],
            cpu="4096", memory="16384", executionRoleArn=self.settings.execution_role,
            runtimePlatform={"cpuArchitecture": "X86_64", "operatingSystemFamily": "LINUX"},
            containerDefinitions=[container], tags=tags(attempt),
        )["taskDefinition"]["taskDefinitionArn"]
        return {
            "cluster": self.settings.cluster, "taskDefinition": definition,
            "launchType": "FARGATE", "platformVersion": "LATEST", "count": 1,
            "clientToken": attempt, "startedBy": attempt, "enableECSManagedTags": True,
            "networkConfiguration": {"awsvpcConfiguration": {
                "assignPublicIp": "ENABLED", "subnets": self.settings.subnets,
                "securityGroups": [self.settings.security_group],
            }},
            "overrides": {"containerOverrides": [{"name": "scope-runner", "environment": [
                {"name": "SCOPE_API_URL", "value": self.settings.api},
                {"name": "SCOPE_ATTEMPT_ID", "value": attempt},
                {"name": "SCOPE_ATTEMPT_DEADLINE_UNIX", "value": str(record["deadline_unix"])},
            ]}]},
            "tags": tags(attempt),
        }

    def launch(self, specification):
        try:
            result = self.ecs.run_task(**specification)
        except Exception as error:
            code = error_code(error)
            if code in {
                "ClientException", "InvalidParameterException", "AccessDeniedException",
                "ClusterNotFoundException", "PlatformUnknownException", "UnsupportedFeatureException",
                "PlatformTaskDefinitionIncompatibilityException", "BlockedException",
            }:
                detail = getattr(error, "response", {}).get("Error", {}).get("Message", "")
                reason = rejection_reason(code, detail) if code == "ClientException" else "permanent"
                raise ProviderRejected(reason, request_id(getattr(error, "response", {}))) from None
            raise
        tasks = result.get("tasks", [])
        if len(tasks) == 1 and tasks[0].get("taskArn"):
            return tasks[0]["taskArn"]
        if not tasks and result.get("failures"):
            failures = result["failures"]
            reasons = [rejection_reason(item.get("reason", ""), item.get("detail", "")) for item in failures]
            reason = reasons[0] if len(set(reasons)) == 1 else "permanent"
            raise ProviderRejected(reason, request_id(result))
        raise Pending("ECS launch requires reconciliation")

    def find(self, attempt):
        # startedBy must be the only ListTasks filter. It selects this attempt's
        # tasks in the fixed cluster, never an ARN supplied by the caller.
        result = []
        token = None
        while True:
            args = {"cluster": self.settings.cluster, "startedBy": attempt}
            if token:
                args["nextToken"] = token
            page = self.ecs.list_tasks(**args)
            result.extend(page.get("taskArns", []))
            token = page.get("nextToken")
            if not token:
                return result

    def stop(self, task):
        try:
            self.ecs.stop_task(cluster=self.settings.cluster, task=task, reason="Scope attempt cleanup")
        except Exception as error:
            if error_code(error) != "ClientException" or "not found" not in str(error).lower():
                raise
            return False
        return True

    def status(self, task):
        result = self.ecs.describe_tasks(cluster=self.settings.cluster, tasks=[task])
        tasks = result.get("tasks", [])
        failures = result.get("failures", [])
        if len(tasks) == 1 and tasks[0].get("taskArn") == task and tasks[0].get("lastStatus") == "STOPPED":
            return "stopped"
        if not tasks and failures and all(item.get("reason", "").upper() == "MISSING" for item in failures):
            # A just-launched task can be temporarily invisible even when ECS
            # already returned its ARN. The lifecycle applies a consistency grace.
            return "missing"
        if failures:
            raise Pending("ECS task status requires reconciliation")
        return "stopping"

    def cleanup(self, record):
        attempt = record["attempt_id"]
        family = "scope-runner-" + attempt
        known = record.get("launch_specification", {}).get("taskDefinition")
        definitions = {known} if known else set()
        token = None
        while True:
            args = {"familyPrefix": family, "status": "ACTIVE", "maxResults": 100}
            if token:
                args["nextToken"] = token
            page = self.ecs.list_task_definitions(**args)
            definitions.update(arn for arn in page.get("taskDefinitionArns", []) if arn.rsplit("/", 1)[-1].rsplit(":", 1)[0] == family)
            token = page.get("nextToken")
            if not token:
                break
        for definition in sorted(definitions):
            self.ecs.deregister_task_definition(taskDefinition=definition)
        try:
            self.secrets.delete_secret(SecretId=self.secret_name(attempt), ForceDeleteWithoutRecovery=True)
        except Exception as error:
            if error_code(error) != "ResourceNotFoundException":
                raise


def registry_host(image):
    reference = image.split("@", 1)[0]
    first, separator, _ = reference.partition("/")
    if separator and ("." in first or ":" in first or first == "localhost"):
        return first.lower()
    return "docker.io"
