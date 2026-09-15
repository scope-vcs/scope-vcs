"""Bounded, conditional object capture. Source credentials never enter recovery archives."""

import hashlib
import os
from pathlib import Path
from urllib.parse import urlsplit

from common import Incomplete, PREFIXES, object_path, required


def source_clients():
    import boto3
    from botocore.config import Config
    clients = {}
    for bucket, prefix in PREFIXES.items():
        if bucket == "cache" and os.environ.get("SCOPE_RECOVERY_INCLUDE_CACHE", "").lower() != "true":
            continue
        endpoint = required(prefix + "_ENDPOINT")
        parsed = urlsplit(endpoint)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.path not in ("", "/") or parsed.query or parsed.fragment:
            raise Incomplete("source bucket must use a verified HTTPS origin")
        style = "path" if os.environ.get(prefix + "_FORCE_PATH_STYLE", "").lower() in ("1", "true", "yes") else "virtual"
        client = boto3.client("s3", endpoint_url=endpoint, region_name=required(prefix + "_REGION"), aws_access_key_id=required(prefix + "_ACCESS_KEY_ID"), aws_secret_access_key=required(prefix + "_SECRET_ACCESS_KEY"), config=Config(connect_timeout=5, read_timeout=60, retries={"total_max_attempts": 3}, s3={"addressing_style": style}))
        clients[bucket] = (client, required(prefix + "_NAME"))
    return clients


def inventory(clients, max_bytes, max_objects):
    result = {}
    total_bytes = 0
    total_objects = 0
    for bucket, (client, name) in clients.items():
        items = []
        for page in client.get_paginator("list_objects_v2").paginate(Bucket=name):
            for value in page.get("Contents", []):
                total_bytes += value["Size"]
                total_objects += 1
                if total_bytes > max_bytes or total_objects > max_objects:
                    raise Incomplete("source inventory exceeds configured recovery cap")
                object_path(bucket, value["Key"])
                items.append({"key": value["Key"], "etag": value["ETag"], "size": value["Size"]})
        result[bucket] = sorted(items, key=lambda item: item["key"])
    return result


def copy_objects(clients, before, destination):
    result = {}
    for bucket, values in before.items():
        client, name = clients[bucket]
        result[bucket] = []
        for item in values:
            path = Path(destination) / object_path(bucket, item["key"])
            path.parent.mkdir(parents=True, exist_ok=True)
            response = client.get_object(Bucket=name, Key=item["key"], IfMatch=item["etag"])
            if response["ContentLength"] != item["size"] or response.get("ETag") != item["etag"]:
                raise Incomplete("source object changed during capture")
            checksum = hashlib.sha256()
            size = 0
            try:
                with path.open("xb") as target:
                    for chunk in response["Body"].iter_chunks(chunk_size=1024**2):
                        size += len(chunk)
                        if size > item["size"]:
                            raise Incomplete("source object exceeded its listed size")
                        checksum.update(chunk)
                        target.write(chunk)
            finally:
                response["Body"].close()
            if size != item["size"]:
                raise Incomplete("source object was truncated")
            result[bucket].append(dict(item, sha256=checksum.hexdigest()))
    return result
