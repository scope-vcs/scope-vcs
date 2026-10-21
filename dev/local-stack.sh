#!/usr/bin/env bash

SLOT_FILE="$STATE_DIR/slot"
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
  if [[ -f "$SLOT_FILE" ]]; then
    cat "$SLOT_FILE"
  else
    echo 0
  fi
}

isolated_database() {
  python3 - "$(local_database_url)" "$SLOT" "$1" <<'EOF'
import re
import sys
from urllib.parse import urlsplit, urlunsplit

url, slot, part = sys.argv[1:]
parts = urlsplit(url)
if parts.hostname not in ("localhost", "127.0.0.1", "::1"):
    sys.exit("DATABASE_URL must target localhost to derive an isolated database")
name = f"{parts.path.lstrip('/')}_stack{slot}"
if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*_stack[0-9]+", name):
    sys.exit(f"cannot derive an isolated database name from DATABASE_URL: {name}")
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
  psql "$(local_database_url)" -XAtq -v ON_ERROR_STOP=1 -c "$1"
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
  mkdir -p "$STACKS_DIR"
  chmod 700 "$STACKS_DIR"
  exec {STACKS_LOCK}>"$STACKS_DIR/lock"
  flock "$STACKS_LOCK"
  "$@"
  exec {STACKS_LOCK}>&-
}

slot_owner() {
  if [[ -f "$STACKS_DIR/slot-$1" ]]; then
    cat "$STACKS_DIR/slot-$1"
  fi
}

claim_is_live() {
  local slot="$1" owner="$2"
  [[ "$(cat "$owner/.tmp/local-dev/slot" 2>/dev/null || true)" == "$slot" ]] &&
    [[ "$(python3 "$ROOT/dev/local-process.py" status "$owner/.tmp/local-dev/expiry.pid")" == running* ]]
}

slot_ports_free() {
  local port
  for port in "$WEB_PORT" "$API_PORT" "$WORKER_PORT" "$MEDIA_PORT" "$MEDIA_WORKER_PORT"; do
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
  if (( SLOT == 0 )); then
    return
  fi
  with_stacks_lock release_owned_slot
  rm -f -- "$SLOT_FILE"
  use_slot 0
}

process_start_time() {
  sed 's/.*) //' "/proc/$1/stat" 2>/dev/null | cut -d' ' -f20
}

write_expiry_runner() {
  local runner="$STATE_DIR/expiry.sh"
  cat > "$runner" <<EOF
#!/usr/bin/env bash
set -euo pipefail
startup_running() {
  [[ "\$(sed 's/.*) //' /proc/$$/stat 2>/dev/null | cut -d' ' -f20)" == "$(process_start_time $$)" ]]
}
sleep $ISOLATED_LIFETIME_SECONDS
while startup_running; do
  sleep 10
done
exec setsid -f $(printf '%q' "$ROOT/dev/scope-dev") down
EOF
  chmod +x "$runner"
}
