"""DynamoDB serializes an attempt across Lambda invocations and retains tombstones."""

from decimal import Decimal

from protocol import Pending


# The deployed Lambda is forcibly terminated at 120 seconds. A stale lock cannot
# be taken over while its previous invocation could still issue AWS mutations.
LOCK_SECONDS = 180


class Journal:
    def __init__(self, table):
        self.table = table

    def acquire(self, attempt, owner, now):
        try:
            result = self.table.update_item(
                Key={"attempt_id": attempt},
                UpdateExpression="SET lock_owner = :owner, lock_until = :until",
                ConditionExpression="attribute_not_exists(lock_until) OR lock_until < :now",
                ExpressionAttributeValues={":owner": owner, ":until": now + LOCK_SECONDS, ":now": now},
                ReturnValues="ALL_NEW",
            )
        except Exception as error:
            if error_code(error) == "ConditionalCheckFailedException":
                raise Pending("another dispatch operation owns this attempt") from None
            raise
        return integers(result["Attributes"])

    def save(self, record, owner):
        self.table.put_item(
            Item=record,
            ConditionExpression="lock_owner = :owner",
            ExpressionAttributeValues={":owner": owner},
        )

    def release(self, attempt, owner):
        try:
            self.table.update_item(
                Key={"attempt_id": attempt},
                UpdateExpression="REMOVE lock_owner, lock_until",
                ConditionExpression="lock_owner = :owner",
                ExpressionAttributeValues={":owner": owner},
            )
        except Exception as error:
            if error_code(error) != "ConditionalCheckFailedException":
                raise


def error_code(error):
    return getattr(error, "response", {}).get("Error", {}).get("Code")


def integers(value):
    # DynamoDB's document API returns Decimal even for ECS's integer count.
    if isinstance(value, Decimal):
        return int(value)
    if isinstance(value, list):
        return [integers(item) for item in value]
    if isinstance(value, dict):
        return {key: integers(item) for key, item in value.items()}
    return value
