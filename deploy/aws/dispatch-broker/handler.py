"""AWS Lambda entrypoint. No function URL; IAM authorizes exact-function invoke."""

import json

import boto3
from botocore.config import Config

from journal import Journal
from lifecycle import Broker
from protocol import Authority
from provider import Provider
from settings import Settings


_broker = None


def handler(event, context):
    global _broker
    if _broker is None:
        settings = Settings()
        # Disable SDK retries: ambiguous mutations remain owned by the journal.
        config = Config(connect_timeout=3, read_timeout=10, retries={"total_max_attempts": 1})
        table = boto3.resource("dynamodb", config=config).Table(settings.table)
        provider = Provider(boto3.client("ecs", config=config), boto3.client("secretsmanager", config=config), settings)
        _broker = Broker(Journal(table), Authority(settings.api, settings.authority_token), provider)
    result = _broker.handle(event, context.aws_request_id)
    print(json.dumps({"event": "dispatch_result", "status": result["status"], "request_id": context.aws_request_id}))
    return result
