"""Append-only publication. Never read, overwrite, or delete recovery object versions."""

import base64
import hashlib
from pathlib import Path

from common import Incomplete, canonical, digest


PART_BYTES = 64 * 1024**2


def put(client, bucket, key, body):
    checksum = base64.b64encode(hashlib.sha256(body).digest()).decode()
    result = client.put_object(Bucket=bucket, Key=key, Body=body, ServerSideEncryption="AES256", ChecksumSHA256=checksum)
    if result.get("VersionId") in (None, "", "null") or result.get("ChecksumSHA256") != checksum:
        raise Incomplete("versioned recovery publication did not confirm its checksum")
    return result["VersionId"]


def upload_archive(client, bucket, key, path):
    with Path(path).open("rb") as incoming:
        first = incoming.read(PART_BYTES)
        if Path(path).stat().st_size <= PART_BYTES:
            return put(client, bucket, key, first)
        upload = client.create_multipart_upload(Bucket=bucket, Key=key, ServerSideEncryption="AES256", ChecksumAlgorithm="SHA256")["UploadId"]
        parts = []
        try:
            body = first
            while body:
                checksum = base64.b64encode(hashlib.sha256(body).digest()).decode()
                number = len(parts) + 1
                result = client.upload_part(Bucket=bucket, Key=key, UploadId=upload, PartNumber=number, Body=body, ChecksumSHA256=checksum)
                if result.get("ChecksumSHA256") != checksum:
                    raise Incomplete("recovery upload part checksum failed")
                parts.append({"PartNumber": number, "ETag": result["ETag"], "ChecksumSHA256": checksum})
                body = incoming.read(PART_BYTES)
            result = client.complete_multipart_upload(Bucket=bucket, Key=key, UploadId=upload, MultipartUpload={"Parts": parts})
            if result.get("VersionId") in (None, "", "null"):
                raise Incomplete("recovery upload returned no immutable version")
            return result["VersionId"]
        except BaseException:
            try:
                client.abort_multipart_upload(Bucket=bucket, Key=key, UploadId=upload)
            except Exception:
                pass
            raise


def publish(client, bucket, capture_id, archive, summary):
    key = "sets/" + capture_id + "/recovery.tar.age"
    version = upload_archive(client, bucket, key, archive)
    complete = dict(summary, archive_key=key, archive_version=version, archive_sha256=digest(archive), archive_bytes=Path(archive).stat().st_size)
    marker = "sets/" + capture_id + "/complete.json"
    marker_version = put(client, bucket, marker, canonical(complete))
    return {"capture_id": capture_id, "complete_key": marker, "complete_version": marker_version, "archive_sha256": complete["archive_sha256"]}


def metric(client, bucket, complete):
    client.put_metric_data(Namespace="Scope/Security/Recovery", MetricData=[{"MetricName": "RecoverySetComplete", "Dimensions": [{"Name": "BucketName", "Value": bucket}], "Value": int(complete), "Unit": "Count"}])
