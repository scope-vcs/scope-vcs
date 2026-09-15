#!/usr/bin/env bash
# Executed only inside the explicit production maintenance service over Railway SSH.
set -euo pipefail
umask 077
: "${DATABASE_URL:?Private maintenance database URL is required}"
export PGCONNECT_TIMEOUT=10
cd "${1:?Isolated scratch directory is required}"
cat > snapshot.psql <<'SQL'
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '30min';
SELECT pg_export_snapshot() AS snapshot \gset
\setenv SCOPE_RECOVERY_SNAPSHOT :snapshot
\! pg_dump --dbname="$DATABASE_URL" --no-password --format=custom --no-owner --no-acl --snapshot="$SCOPE_RECOVERY_SNAPSHOT" --file=database.dump && touch dump.ok
\o references.jsonl
\i references-json.sql
\o captured-at.txt
SELECT transaction_timestamp()::text;
\o
ROLLBACK;
SQL
psql --dbname="$DATABASE_URL" --no-psqlrc --no-password -qAt -v ON_ERROR_STOP=1 -f snapshot.psql > /dev/null
test -f dump.ok && test -s database.dump && test -f references.jsonl && test -s captured-at.txt
test "$(stat -c %s database.dump)" -le 1073741824
tar -czf - database.dump references.jsonl captured-at.txt
