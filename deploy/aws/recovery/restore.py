"""Decrypt and verify a recovery set, optionally restoring only an empty local drill DB."""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

from archive import decrypt
from common import Incomplete, digest, keys, required
from verify import verify


def restore_database(url, root, rebuild_cache):
    import psycopg
    from psycopg.conninfo import conninfo_to_dict
    parameters = conninfo_to_dict(url)
    host = parameters.get("host", "")
    if host not in ("", "localhost", "127.0.0.1", "::1") and not host.startswith("/"):
        raise Incomplete("restore is restricted to a local database")
    if not parameters.get("dbname", "").startswith("scope_recovery_drill_"):
        raise Incomplete("restore database name must start with scope_recovery_drill_")
    with psycopg.connect(url) as connection:
        if connection.execute("SELECT inet_server_addr()::text").fetchone()[0] not in (None, "127.0.0.1/32", "::1/128", "127.0.0.1", "::1"):
            raise Incomplete("restore server is not local")
        if connection.execute("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%'").fetchone()[0]:
            raise Incomplete("restore requires an empty database")
    environment = {"PATH": os.environ.get("PATH", ""), "PGCONNECT_TIMEOUT": "10"}
    for option, name in {"host": "PGHOST", "hostaddr": "PGHOSTADDR", "port": "PGPORT", "user": "PGUSER", "password": "PGPASSWORD", "sslmode": "PGSSLMODE", "sslrootcert": "PGSSLROOTCERT", "sslcert": "PGSSLCERT", "sslkey": "PGSSLKEY"}.items():
        if option in parameters:
            environment[name] = parameters[option]
    result = subprocess.run(["pg_restore", "--no-password", "--exit-on-error", "--no-owner", "--no-acl", "--dbname=" + parameters["dbname"], str(Path(root) / "database.dump")],
                            env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=1800)
    if result.returncode:
        raise Incomplete("isolated database restore failed")
    with psycopg.connect(url) as connection:
        if rebuild_cache:
            connection.execute(Path(__file__).with_name("rebuild-cache.sql").read_text())
        tables = connection.execute("SELECT count(*) FROM pg_tables WHERE schemaname='public'").fetchone()[0]
    return tables


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("--identity", required=True)
    parser.add_argument("--destination", required=True)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--restore-database", action="store_true", help="Use SCOPE_RECOVERY_DRILL_DATABASE_URL, only an empty local scope_recovery_drill_* database")
    args = parser.parse_args()
    os.umask(0o077)
    try:
        if digest(args.archive) != args.expected_sha256:
            raise Incomplete("encrypted archive checksum failed")
        decrypt(args.archive, args.identity, args.destination)
        root = Path(args.destination)
        manifest = json.loads((root / "manifest.json").read_bytes())
        if manifest["version"] != 1 or digest(root / "database.dump") != manifest["database"]["dump_sha256"]:
            raise Incomplete("database snapshot checksum or recovery format failed")
        result = verify(root, manifest["inventory"], manifest["database"]["references"], keys(root / "keys.json"))
        result["excluded_rebuildable_buckets"] = manifest["database"]["excluded_rebuildable_buckets"]
        if args.restore_database:
            result["restored_tables"] = restore_database(required("SCOPE_RECOVERY_DRILL_DATABASE_URL"), root, "cache" in result["excluded_rebuildable_buckets"])
        print(json.dumps(result, sort_keys=True))
    except Exception:
        print("Recovery proof failed; do not start restored application services.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
