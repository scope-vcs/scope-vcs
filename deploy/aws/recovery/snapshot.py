"""Capture metadata and required object references without blocking application writes."""

import os
import subprocess
from pathlib import Path

from common import Incomplete, digest


def snapshot(url, destination, include_cache=False):
    import psycopg
    from psycopg.rows import dict_row

    with psycopg.connect(url, row_factory=dict_row, autocommit=True) as connection:
        connection.execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        connection.execute("SET LOCAL statement_timeout = '30min'")
        exported = connection.execute("SELECT pg_export_snapshot() AS id").fetchone()["id"]
        from psycopg.conninfo import conninfo_to_dict
        parameters = conninfo_to_dict(url)
        environment = {"PATH": os.environ.get("PATH", ""), "PGCONNECT_TIMEOUT": "10"}
        for option, name in {"host": "PGHOST", "hostaddr": "PGHOSTADDR", "port": "PGPORT", "user": "PGUSER", "password": "PGPASSWORD", "sslmode": "PGSSLMODE", "sslrootcert": "PGSSLROOTCERT", "sslcert": "PGSSLCERT", "sslkey": "PGSSLKEY"}.items():
            if option in parameters:
                environment[name] = parameters[option]
        result = subprocess.run(
            ["pg_dump", "--dbname=" + parameters["dbname"], "--no-password", "--format=custom", "--no-owner", "--no-acl", "--snapshot=" + exported, "--file=" + str(destination)],
            env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=1800,
        )
        if result.returncode:
            raise Incomplete("database snapshot failed")
        rows = connection.execute(Path(__file__).with_name("references.sql").read_text()).fetchall()
        unknown = connection.execute("SELECT count(*) AS count FROM scope_object_references WHERE NOT (object_key::jsonb ?| ARRAY['BlobSha256','GitBundleSha256','GitBlob'])").fetchone()["count"]
        if unknown:
            raise Incomplete("database contains an unsupported content reference")
        captured_at = connection.execute("SELECT transaction_timestamp()::text AS captured_at").fetchone()["captured_at"]
        connection.rollback()
    return metadata(rows, captured_at, destination, include_cache)


def metadata(rows, captured_at, destination, include_cache=False):
    required = []
    excluded_cache_objects = 0
    for row in rows:
        if row["bucket"] == "cache" and not include_cache:
            excluded_cache_objects += 1
            continue
        if row["kind"] == "content":
            reference = row.pop("content_ref")
            if not isinstance(reference, dict) or len(reference) != 1 or not set(reference) <= {"GitBlob", "BlobSha256", "GitBundleSha256"}:
                raise Incomplete("database contains an unsupported content reference")
            if "GitBlob" in reference:
                continue
            kind = "BlobSha256" if "BlobSha256" in reference else "GitBundleSha256"
            row["key"] = ("objects/blobs/" if kind == "BlobSha256" else "objects/git-bundles/") + reference[kind]
            row["sha256"] = reference[kind]
        else:
            row.pop("content_ref")
        required.append(row)
    return {"captured_at": captured_at, "dump_sha256": digest(destination), "references": required,
            "excluded_rebuildable_buckets": [] if include_cache else ["cache"], "excluded_cache_objects": excluded_cache_objects}
