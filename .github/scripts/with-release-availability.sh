#!/usr/bin/env bash
set -uo pipefail

usage='usage: with-release-availability.sh CONFIG_JSON OUTPUT_DIRECTORY -- COMMAND [ARG...]'
if (($# < 4)) || [[ "$3" != '--' ]]; then
  echo "$usage" >&2
  exit 2
fi

config_input="$1"
output_input="$2"
shift 3
if [[ ! -f "$config_input" ]]; then
  echo "availability config does not exist: $config_input" >&2
  exit 2
fi
config_path="$(realpath "$config_input")"
mkdir -p "$output_input" || exit 2
output_dir="$(realpath "$output_input")"
if [[ "$output_dir" == '/' || -L "$output_input" ]]; then
  echo "availability output must be a specific physical directory" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
events_path="$output_dir/availability.ndjson"
summary_path="$output_dir/availability-summary.json"
ready_path="$output_dir/availability.ready.json"
stop_path="$output_dir/availability.stop"
log_path="$output_dir/availability-process.log"
for path in "$events_path" "$summary_path" "$ready_path" "$stop_path" "$log_path"; do
  if [[ -e "$path" ]]; then
    echo "availability output already exists: $path" >&2
    exit 2
  fi
done

baseline_seconds="${SCOPE_RELEASE_BASELINE_SECONDS:-60}"
post_teardown_seconds="${SCOPE_RELEASE_POST_TEARDOWN_SECONDS:-120}"
ready_timeout_seconds="${SCOPE_RELEASE_READY_TIMEOUT_SECONDS:-30}"
skip_baseline="${SCOPE_RELEASE_SKIP_BASELINE:-0}"
for setting in baseline_seconds post_teardown_seconds ready_timeout_seconds; do
  value="${!setting}"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "$setting must be a non-negative integer" >&2
    exit 2
  fi
done
if [[ "$skip_baseline" != 0 && "$skip_baseline" != 1 ]]; then
  echo "SCOPE_RELEASE_SKIP_BASELINE must be 0 or 1" >&2
  exit 2
fi
if [[ "$skip_baseline" == 1 ]] && ! node --input-type=module - "$config_path" <<'NODE'
import { readFileSync } from "node:fs";
const config = JSON.parse(readFileSync(process.argv[2], "utf8"));
if (config.mode !== "maintenance") process.exit(1);
NODE
then
  echo "SCOPE_RELEASE_SKIP_BASELINE is only valid for maintenance recovery" >&2
  exit 2
fi
umask 077

monitor_pid=''
monitor_stopped=0
stop_monitor() {
  if [[ -z "$monitor_pid" || "$monitor_stopped" == 1 ]]; then
    return
  fi
  : > "$stop_path"
  wait "$monitor_pid"
  monitor_status=$?
  monitor_stopped=1
}
cleanup() {
  if [[ -n "$monitor_pid" && "$monitor_stopped" == 0 ]]; then
    : > "$stop_path"
    wait "$monitor_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

node "$script_dir/release-availability.mjs" \
  --config "$config_path" \
  --events "$events_path" \
  --summary "$summary_path" \
  --ready-file "$ready_path" \
  --stop-file "$stop_path" \
  > "$log_path" 2>&1 &
monitor_pid=$!

ready_waited=0
while [[ ! -f "$ready_path" ]]; do
  if ! kill -0 "$monitor_pid" 2>/dev/null; then
    wait "$monitor_pid"
    status=$?
    monitor_stopped=1
    echo "availability monitor exited before its ready marker (status $status)" >&2
    exit "$status"
  fi
  if ((ready_waited >= ready_timeout_seconds * 10)); then
    echo "availability monitor did not become ready within ${ready_timeout_seconds}s" >&2
    exit 1
  fi
  sleep 0.1
  ((ready_waited += 1))
done

wait_while_monitoring() {
  local seconds="$1"
  local elapsed=0
  while ((elapsed < seconds)); do
    if ! kill -0 "$monitor_pid" 2>/dev/null; then
      return 1
    fi
    sleep 1
    ((elapsed += 1))
  done
}

if [[ "$skip_baseline" == 0 ]] && ! wait_while_monitoring "$baseline_seconds"; then
  echo "availability monitor exited during the baseline" >&2
  exit 1
fi
minimum_samples="$baseline_seconds"
if ((minimum_samples < 1)); then minimum_samples=1; fi
if [[ "$skip_baseline" == 0 ]] && ! node --input-type=module - "$events_path" "$minimum_samples" <<'NODE'
import { readFileSync } from "node:fs";
const [, , path, minimumText] = process.argv;
const events = readFileSync(path, "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
const samples = new Set(events.map(({ sample }) => sample));
if (events.some(({ ok }) => !ok)) process.exit(1);
if (samples.size < Number(minimumText)) process.exit(1);
NODE
then
  echo "availability baseline contained a failed request or too few samples" >&2
  stop_monitor
  exit 1
fi

"$@"
command_status=$?
if ((command_status == 0)); then
  if ! wait_while_monitoring "$post_teardown_seconds"; then
    echo "availability monitor exited during the post-teardown observation" >&2
  fi
fi

stop_monitor
if ((command_status != 0)); then
  exit "$command_status"
fi
exit "$monitor_status"
