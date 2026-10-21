#!/usr/bin/env bash

SLOT_FILE="$STATE_DIR/slot"
CLAIM_FILE="$STATE_DIR/claim"
STACKS_DIR="${XDG_RUNTIME_DIR:-/tmp}/scope-dev-stacks"
ISOLATED_SLOTS=(1 2)
ISOLATED_LIFETIME_SECONDS=3600

use_slot() {
  SLOT="$1"
  local offset=$((SLOT * 10))
  WEB_PORT=$((3000 + offset))
  API_PORT=$((8080 + offset))
  WORKER_PORT=$((8081 + offset))
  MEDIA_PORT=$((8083 + offset))
  MEDIA_WORKER_PORT=$((8084 + offset))
  LOCAL_WEB_ORIGIN="http://localhost:$WEB_PORT"
  LOCAL_API_ORIGIN="http://localhost:$API_PORT"
}

recorded_slot() {
  local slot
  slot="$(cat "$SLOT_FILE" 2>/dev/null || echo 0)"
  if [[ "$slot" != 0 && "$(slot_owner "$slot")" != "$ROOT" ]]; then
    slot=0
  fi
  echo "$slot"
}

isolated_database() {
  python3 - "$(local_database_url)" "$SLOT" "$1" <<'EOF'
import re
import sys
from urllib.parse import parse_qsl, urlsplit, urlunsplit

url, slot, part = sys.argv[1:]
parts = urlsplit(url)
if parts.scheme not in ("postgres", "postgresql") or parts.hostname not in ("localhost", "127.0.0.1", "::1"):
    sys.exit("DATABASE_URL must be a PostgreSQL URL on localhost to derive an isolated database")
target_neutral = {"port", "user", "password", "sslmode", "sslrootcert", "sslcert", "sslkey", "application_name"}
overrides = sorted({key for key, _ in parse_qsl(parts.query, keep_blank_values=True)} - target_neutral)
if overrides:
    sys.exit(f"DATABASE_URL parameters cannot choose the isolated database's host or name: {', '.join(overrides)}")
name = f"{parts.path.lstrip('/')}_stack{slot}"
if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*_stack[0-9]+", name) or len(name.encode()) > 63:
    sys.exit(f"cannot derive an isolated database name of at most 63 bytes from DATABASE_URL: {name}")
print(name if part == "name" else urlunsplit(parts._replace(path=f"/{name}")))
EOF
}

stack_database_url() {
  if (( SLOT == 0 )) || [[ -z "$(local_database_url)" ]]; then
    local_database_url
  else
    isolated_database url
  fi
}

run_on_configured_database() {
  env -u PGHOSTADDR -u PGSERVICE psql "$(local_database_url)" -XAtq -v ON_ERROR_STOP=1 -c "$1"
}

drop_isolated_database() {
  run_on_configured_database "DROP DATABASE IF EXISTS \"$(isolated_database name)\" WITH (FORCE)"
}

recreate_owned_database() {
  if [[ "$(slot_owner "$SLOT")" != "$ROOT" ]]; then
    echo "isolated stack $SLOT expired before startup finished; run ./dev/scope-dev up --isolated again" >&2
    exit 1
  fi
  drop_isolated_database
  run_on_configured_database "CREATE DATABASE \"$(isolated_database name)\""
}

with_stacks_lock() {
  if [[ -n "${STACKS_LOCK:-}" ]]; then
    "$@"
    return
  fi
  mkdir -p "$STACKS_DIR"
  chmod 700 "$STACKS_DIR"
  exec {STACKS_LOCK}>"$STACKS_DIR/lock"
  flock "$STACKS_LOCK"
  "$@"
  exec {STACKS_LOCK}>&-
  unset STACKS_LOCK
}

slot_owner() {
  if [[ -f "$STACKS_DIR/slot-$1" ]]; then
    cat "$STACKS_DIR/slot-$1"
  fi
}

claim_is_live() {
  local slot="$1" owner="$2"
  [[ "$(cat "$owner/.tmp/local-dev/slot" 2>/dev/null || true)" == "$slot" ]] || return 1
  [[ "$(python3 "$ROOT/dev/local-process.py" status "$owner/.tmp/local-dev/expiry.pid")" == running* ]] ||
    { [[ -f "$owner/dev/scope-dev" ]] && ! flock -n "$owner/dev/scope-dev" true; }
}

slot_ports_free() {
  local port ports=("$API_PORT" "$WORKER_PORT")
  if (( WITH_MEDIA )); then
    ports+=("$MEDIA_PORT" "$MEDIA_WORKER_PORT")
  fi
  if [[ "${SCOPE_DEV_HEADLESS_E2E:-0}" != "1" ]]; then
    ports+=("$WEB_PORT")
  fi
  for port in "${ports[@]}"; do
    [[ -z "$(port_owner "$port")" ]] || return 1
  done
}

claim_free_slot() {
  local slot owner
  for slot in "${ISOLATED_SLOTS[@]}"; do
    owner="$(slot_owner "$slot")"
    if [[ -n "$owner" ]] && claim_is_live "$slot" "$owner"; then
      continue
    fi
    use_slot "$slot"
    slot_ports_free || continue
    mkdir -p "$STATE_DIR"
    printf '%s\n' "$slot" > "$SLOT_FILE"
    CLAIM_TOKEN="$(cat /proc/sys/kernel/random/uuid)"
    printf '%s\n' "$CLAIM_TOKEN" > "$CLAIM_FILE"
    write_expiry_runner
    start_service expiry {STACKS_LOCK}>&-
    printf '%s\n' "$ROOT" > "$STACKS_DIR/slot-$slot"
    return
  done
  use_slot 0
  echo "every isolated stack slot is in use; run ./dev/scope-dev down in a checkout that no longer needs its stack:" >&2
  for slot in "${ISOLATED_SLOTS[@]}"; do
    echo "  slot $slot: $(slot_owner "$slot")" >&2
  done
  exit 1
}

release_owned_slot() {
  if [[ "$(slot_owner "$SLOT")" == "$ROOT" ]]; then
    if ! drop_isolated_database; then
      echo "could not drop $(isolated_database name); drop it once Postgres is reachable" >&2
    fi
    rm -f -- "$STACKS_DIR/slot-$SLOT"
  fi
}

claim_isolated_slot() {
  with_stacks_lock claim_free_slot
}

release_isolated_stack() {
  if (( SLOT != 0 )); then
    with_stacks_lock release_owned_slot
  fi
  rm -f -- "$SLOT_FILE" "$CLAIM_FILE"
  CLAIM_TOKEN=""
  use_slot 0
}

write_expiry_runner() {
  local runner="$STATE_DIR/expiry.sh"
  cat > "$runner" <<EOF
#!/usr/bin/env bash
set -euo pipefail
sleep $ISOLATED_LIFETIME_SECONDS
SCOPE_DEV_EXPIRING_CLAIM=$CLAIM_TOKEN exec setsid -f $(printf '%q' "$ROOT/dev/scope-dev") down
EOF
  chmod +x "$runner"
}
