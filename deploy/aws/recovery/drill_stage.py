"""Runs inside the sealed drill namespace: fills new buckets, restores the database, adds drill users.

The host orchestrator, drill.py, supplies every value through the environment.
"""

import hashlib
import json
import os
import time
import urllib.parse
from pathlib import Path

from common import Incomplete, digest, object_path, required
from restore import restore_database

BUCKETS = {"objects": "scope-drill-objects", "media": "scope-drill-media", "cache": "scope-drill-cache"}
NO_PERMISSIONS = {"can_push": False, "can_change_file_visibility": False}


def upload(root, inventory):
    import boto3
    from botocore.config import Config
    s3 = boto3.client("s3", endpoint_url=required("SCOPE_DRILL_S3_ENDPOINT"),
                      aws_access_key_id=required("SCOPE_DRILL_S3_ACCESS_KEY"),
                      aws_secret_access_key=required("SCOPE_DRILL_S3_SECRET_KEY"),
                      region_name="us-east-1", config=Config(s3={"addressing_style": "path"}))
    counts = {}
    for bucket, items in inventory.items():
        s3.create_bucket(Bucket=BUCKETS[bucket])
        for item in items:
            # restore.py verified every object against the inventory and database references.
            with (root / object_path(bucket, item["key"])).open("rb") as body:
                s3.put_object(Bucket=BUCKETS[bucket], Key=item["key"], Body=body, IfNoneMatch="*")
        counts[bucket] = len(items)
    return counts


def token_hash(token):
    return "sha256:" + hashlib.sha256(token.encode()).hexdigest()


def add_identities(url, repo, users):
    """Adds drill-only users with CLI sessions; only the member joins the restored repository."""
    import psycopg
    from psycopg.types.json import Jsonb
    now = int(time.time())
    with psycopg.connect(url) as connection:
        if connection.execute("SELECT 1 FROM scope_repositories WHERE id = %s", (repo,)).fetchone() is None:
            raise Incomplete("drill repository is not in the restored database")
        for user in users:
            user_id = "scope_usr_recovery_drill_" + user["role"]
            connection.execute("INSERT INTO scope_users (id, handle, email, email_verified) VALUES (%s, %s, %s, true)",
                               (user_id, user["handle"], user["handle"] + "@recovery-drill.invalid"))
            connection.execute("INSERT INTO scope_cli_sessions (id, token_hash, user_id, label, created_at_unix, expires_at_unix)"
                               " VALUES (%s, %s, %s, 'recovery drill', %s, %s)",
                               ("scope_cli_session_recovery_drill_" + user["role"], token_hash(user["token"]), user_id, now, now + 86400))
            if user["role"] == "member":
                connection.execute("INSERT INTO scope_repository_members (repo_id, user_id, permissions, created_at_unix, updated_at_unix)"
                                   " VALUES (%s, %s, %s, %s, %s)",
                                   (repo, user_id, Jsonb({**NO_PERMISSIONS, "can_push": True}), now, now))


def wait_for(check, what, seconds=180):
    deadline = time.monotonic() + seconds
    while True:
        try:
            if check():
                return
        except Exception:
            pass
        if time.monotonic() > deadline:
            raise Incomplete(f"{what} did not become ready")
        time.sleep(1)


def wait_for_database():
    import psycopg
    url = required("SCOPE_RECOVERY_DRILL_DATABASE_URL")
    wait_for(lambda: psycopg.connect(url).close() is None, "restored database server")


def request(method, url, token=None, body=None):
    """Returns (status, body bytes); HTTP errors are results, not exceptions."""
    import urllib.error
    import urllib.request
    headers = {"x-scope-cli-protocol": "1"}
    if token:
        headers["authorization"] = "Bearer " + token
    if body is not None:
        headers["content-type"] = "application/json"
        body = json.dumps(body).encode()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, body, headers, method=method), timeout=30) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_for_services():
    for name, url in {"API": "http://127.0.0.1:8080/readyz", "worker": "http://127.0.0.1:8081/readyz",
                      "media service": "http://127.0.0.1:8083/readyz"}.items():
        wait_for(lambda: 200 <= request("GET", url)[0] < 300, name)


READABLE_MEDIA = """
SELECT a.repository_id, a.request_id, a.id, m.sha256
FROM scope_request_media_attachments a
JOIN scope_request_media_manifests m ON m.id = a.original_manifest_id
WHERE a.state = 'Ready' AND a.original_validated_at_unix IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM scope_request_media_cleanup_jobs c WHERE c.attachment_id = a.id)
ORDER BY a.repository_id = %s DESC, a.id LIMIT 5
"""


def media_canary(api, url, repo, token):
    """Reads one restored original through an API grant and compares it with its manifest."""
    import psycopg
    from psycopg.types.json import Jsonb
    with psycopg.connect(url) as connection:
        candidates = connection.execute(READABLE_MEDIA, (repo,)).fetchall()
        if not candidates:
            return "failed: the snapshot has no ready media attachment to read back"
        now = int(time.time())
        for repository in {candidate[0] for candidate in candidates} - {repo}:
            connection.execute("INSERT INTO scope_repository_members (repo_id, user_id, permissions, created_at_unix, updated_at_unix)"
                               " VALUES (%s, 'scope_usr_recovery_drill_member', %s, %s, %s)",
                               (repository, Jsonb(NO_PERMISSIONS), now, now))
    for repository, request_id, attachment, sha256 in candidates:
        status, body = request("POST", f"{api}/v1/repos/{repository}/requests/{request_id}/attachments/{attachment}/media-grant",
                               token, {"target": {"kind": "original"}})
        if status != 200:
            continue
        status, content = request("GET", json.loads(body)["media_url"])
        if status == 200 and hashlib.sha256(content).hexdigest() == sha256:
            return "passed: a restored original matched its manifest checksum"
        return f"failed: restored original returned HTTP {status} or a different checksum"
    return "failed: the member could not obtain a grant for any restored original"


def http_canaries():
    api, repo = required("SCOPE_DRILL_API"), required("SCOPE_DRILL_REPO")
    path = urllib.parse.quote(required("SCOPE_DRILL_PRIVATE_PATH"))
    content = f"{api}/v1/repos/{repo}/files/content?path={path}"
    member = request("GET", content, required("SCOPE_DRILL_MEMBER_TOKEN"))[0]
    outsider = request("GET", content, required("SCOPE_DRILL_OUTSIDER_TOKEN"))[0]
    results = {
        "private_read": "passed" if member == 200 else f"failed: member received HTTP {member}",
        "private_hidden": "passed" if outsider == 404 else f"failed: non-member received HTTP {outsider}",
        "media": media_canary(api, required("SCOPE_RECOVERY_DRILL_DATABASE_URL"), repo, required("SCOPE_DRILL_MEMBER_TOKEN")),
    }
    print(json.dumps(results))


def main():
    os.umask(0o077)
    root = Path(required("SCOPE_DRILL_RESTORED"))
    manifest = json.loads((root / "manifest.json").read_bytes())
    if manifest["version"] != 1 or digest(root / "database.dump") != manifest["database"]["dump_sha256"]:
        raise Incomplete("database snapshot checksum or recovery format failed")
    url = required("SCOPE_RECOVERY_DRILL_DATABASE_URL")
    excluded = manifest["database"]["excluded_rebuildable_buckets"]
    result = {"uploaded_objects": upload(root, manifest["inventory"]),
              "restored_tables": restore_database(url, root, "cache" in excluded)}
    add_identities(url, required("SCOPE_DRILL_REPO"), json.loads(required("SCOPE_DRILL_USERS")))
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
