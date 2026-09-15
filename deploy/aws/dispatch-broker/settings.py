import os
import re
from urllib.parse import urlsplit


class Settings:
    def __init__(self):
        self.region = required("AWS_REGION")
        self.cluster = required("SCOPE_ECS_CLUSTER_ARN")
        self.subnets = required("SCOPE_ECS_SUBNET_IDS").split(",")
        self.security_group = required("SCOPE_ECS_SECURITY_GROUP_ID")
        self.execution_role = required("SCOPE_ECS_EXECUTION_ROLE_ARN")
        self.log_group = required("SCOPE_ECS_LOG_GROUP")
        self.registry_secret = os.environ.get("SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN", "")
        self.registry_host = os.environ.get("SCOPE_REGISTRY_CREDENTIALS_HOST", "").lower()
        if bool(self.registry_secret) != bool(self.registry_host):
            raise ValueError("registry credential ARN and host must be configured together")
        if self.registry_host and not re.fullmatch(r"[a-z0-9.-]+(?::[0-9]{1,5})?", self.registry_host):
            raise ValueError("registry credential host must be an exact registry host")
        self.api = required("SCOPE_PUBLIC_API_URL").rstrip("/")
        parsed = urlsplit(self.api)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
            raise ValueError("SCOPE_PUBLIC_API_URL must be an HTTPS origin")
        self.authority_token = required("SCOPE_DISPATCH_BROKER_TOKEN")
        self.table = required("SCOPE_DISPATCH_JOURNAL_TABLE")


def required(name):
    value = os.environ.get(name, "").strip()
    if not value:
        raise ValueError(name + " is required")
    return value
