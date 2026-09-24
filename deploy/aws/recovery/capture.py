"""Capture online without application downtime; fail on incomplete or changing state."""

import datetime
import json
import os
import re
import sys
import tempfile
import uuid
from pathlib import Path

from archive import encrypt
from common import Incomplete, MAX_BYTES, MAX_OBJECTS, digest, keys, required, write_json
from objects import copy_objects, inventory, source_clients
from publish import metric, publish
from snapshot import snapshot
from verify import verify


def capture(clients, url, escrow, recipient, destination, source_sha, max_bytes=MAX_BYTES, max_objects=MAX_OBJECTS, snapshot_provider=None):
    # A fresh DB snapshot is required for each retry. Application writes continue.
    with tempfile.TemporaryDirectory(prefix="scope-recovery-") as scratch:
        root = Path(scratch)
        before = inventory(clients, max_bytes, max_objects)
        database = (snapshot_provider(root / "database.dump", include_cache="cache" in clients)
                    if snapshot_provider else snapshot(url, root / "database.dump", include_cache="cache" in clients))
        if (root / "database.dump").stat().st_size > 1024**3:
            raise Incomplete("database dump exceeds the one-GiB capture cap")
        objects = copy_objects(clients, before, root)
        if before != inventory(clients, max_bytes, max_objects):
            raise Incomplete("source inventory changed during capture")
        proof = verify(root, objects, database["references"], escrow)
        write_json(root / "keys.json", escrow)
        manifest = {"version": 1, "source_sha": source_sha, "database": database, "inventory": objects, "proof": proof,
                    "key_names": sorted(escrow), "tool_sha256": {path.name: digest(path) for path in sorted(Path(__file__).parent.glob("*.py"))}}
        write_json(root / "manifest.json", manifest)
        encrypt(root, recipient, destination)
        return {"version": 1, "captured_at": database["captured_at"], "source_sha": source_sha,
                "excluded_rebuildable_buckets": database["excluded_rebuildable_buckets"],
                "object_count": sum(len(values) for values in objects.values()),
                "object_bytes": sum(item["size"] for values in objects.values() for item in values), **proof}


def failure_reason(error):
    # Exceptions from providers can embed credentials or private object keys. Incomplete messages
    # are fixed strings in this tool, so only they are printed; anything else is named by type.
    return str(error) if isinstance(error, Incomplete) else type(error).__name__


def main(snapshot_provider=None):
    import boto3
    from botocore.config import Config
    os.umask(0o077)
    bucket = required("SCOPE_RECOVERY_BUCKET")
    region = required("AWS_REGION")
    config = Config(connect_timeout=5, read_timeout=60, retries={"total_max_attempts": 3})
    s3 = boto3.client("s3", region_name=region, config=config)
    cloudwatch = boto3.client("cloudwatch", region_name=region, config=config)
    try:
        source_sha = required("SCOPE_RECOVERY_SOURCE_SHA")
        if not re.fullmatch(r"[0-9a-f]{40,64}", source_sha):
            raise Incomplete("source revision must be a Git hash")
        clients = source_clients()
        escrow = keys(required("SCOPE_RECOVERY_KEYS_FILE"))
        max_bytes = int(os.environ.get("SCOPE_RECOVERY_MAX_BYTES", MAX_BYTES))
        max_objects = int(os.environ.get("SCOPE_RECOVERY_MAX_OBJECTS", MAX_OBJECTS))
        if not 0 < max_bytes <= MAX_BYTES or not 0 < max_objects <= MAX_OBJECTS:
            raise Incomplete("capture caps may only be reduced")
        capture_id = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex
        with tempfile.TemporaryDirectory(prefix="scope-encrypted-recovery-") as work:
            archive = Path(work) / "recovery.tar.age"
            for attempt in range(3):
                try:
                    summary = capture(clients, required("SCOPE_RECOVERY_DATABASE_URL") if snapshot_provider is None else "", escrow, required("SCOPE_RECOVERY_RECIPIENT"), archive, source_sha, max_bytes, max_objects, snapshot_provider)
                    break
                except Exception:
                    archive.unlink(missing_ok=True)
                    if attempt == 2:
                        raise
            receipt = publish(s3, bucket, capture_id, archive, summary)
        metric(cloudwatch, bucket, True)
        print(json.dumps(receipt, sort_keys=True))
    except Exception as error:
        try:
            metric(cloudwatch, bucket, False)
        except Exception:
            pass
        print(f"Recovery capture failed ({failure_reason(error)}); no successful completion metric was emitted.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
