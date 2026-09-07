#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/scope-staging-git-smoke-test.XXXXXX")"
trap 'rm -rf -- "$test_root"' EXIT
fake_bin="$test_root/bin"
smoke_dir="$test_root/smoke"
token_path="$smoke_dir/exchange-token"
trace_path="$test_root/trace"
mkdir -m 0700 "$fake_bin" "$smoke_dir"
printf '%s\n' 'scope_otc_do_not_log_this_value' > "$token_path"
chmod 0600 "$token_path"

cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=''
headers=''
while (($#)); do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --dump-header) headers="$2"; shift 2 ;;
    --write-out) shift 2 ;;
    *) shift ;;
  esac
done
if [[ "${FAKE_ROUTER_DIRECT:-1}" = '1' ]]; then
  printf 'x-scope-git-router: 1\r\n\r\n' > "$headers"
  status=200
else
  printf 'location: https://api-staging.example.test/git/public/dev/update-demo/info/refs\r\n\r\n' > "$headers"
  status=302
fi
printf 'git discovery' > "$output"
printf 'curl-router\n' >> "$TRACE_PATH"
printf '%s' "$status"
EOF

cat > "$fake_bin/scope" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  login)
    test "$2" = '--exchange-file'
    test -s "$3"
    if [[ -n "${FAKE_SCOPE_SESSION_TOKEN:-}" ]]; then
      mkdir -p "$XDG_CONFIG_HOME/scope/sessions"
      printf '%s\n' "$FAKE_SCOPE_SESSION_TOKEN" > "$XDG_CONFIG_HOME/scope/sessions/session"
      chmod 0600 "$XDG_CONFIG_HOME/scope/sessions/session"
    fi
    printf 'scope-login-file\n' >> "$TRACE_PATH"
    ;;
  clone)
    destination="$3"
    mkdir -p "$destination/internal"
    printf 'initial\n' > "$destination/README.md"
    printf 'private\n' > "$destination/internal/notes.md"
    printf '%s/git/permissioned/%s\n' "$SCOPE_GIT_ROUTER_URL" "$2" > "$destination/.origin"
    printf 'scope-clone\n' >> "$TRACE_PATH"
    ;;
  push)
    printf 'scope-push\n' >> "$TRACE_PATH"
    ;;
  *) exit 2 ;;
esac
EOF

cat > "$fake_bin/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" = '-c' && "$3" = 'clone' ]]; then
  destination="$6"
  mkdir -p "$destination"
  printf 'initial\n' > "$destination/README.md"
  printf '%s\n' "$5" > "$destination/.origin"
  printf 'git-public-clone\n' >> "$TRACE_PATH"
  exit 0
fi
test "$1" = '-C'
directory="$2"
shift 2
case "$1" in
  remote)
    cat "$directory/.origin"
    ;;
  fetch)
    printf 'git-fetch\n' >> "$TRACE_PATH"
    ;;
  add) ;;
  -c) ;;
  rev-parse)
    printf '1111111111111111111111111111111111111111\n'
    ;;
  show)
    printf 'initial\nScope staging router smoke test-sha\n'
    ;;
  *) exit 2 ;;
esac
EOF

chmod 0755 "$fake_bin/curl" "$fake_bin/git" "$fake_bin/scope"
export TRACE_PATH="$trace_path"
export PATH="$fake_bin:$PATH"
output="$test_root/output"
SCOPE_API_URL='https://api-staging.example.test' \
  SCOPE_GIT_ROUTER_URL='https://router-staging.example.test' \
  SCOPE_CLI_BINARY="$fake_bin/scope" \
  SCOPE_EXCHANGE_TOKEN_PATH="$token_path" \
  SCOPE_GIT_SMOKE_DIR="$smoke_dir" \
  GITHUB_SHA='test-sha' \
  bash "$repo_root/.github/scripts/staging-git-smoke.sh" > "$output" 2>&1

test ! -e "$smoke_dir"
test "$(sed -n '1p' "$trace_path")" = 'curl-router'
test "$(sed -n '2p' "$trace_path")" = 'git-public-clone'
test "$(sed -n '3p' "$trace_path")" = 'scope-login-file'
test "$(sed -n '4p' "$trace_path")" = 'scope-clone'
grep -Fxq 'scope-push' "$trace_path"
if grep -Fq 'scope_otc_do_not_log_this_value' "$output" "$trace_path"; then
  echo "staging Git smoke exposed its exchange token" >&2
  exit 1
fi

cat > "$test_root/media-smoke.mjs" <<'EOF'
import { appendFileSync, writeFileSync } from 'node:fs'
import assert from 'node:assert/strict'
const args = process.argv.slice(2)
const value = (name) => args[args.indexOf(name) + 1]
assert.equal(process.env.SCOPE_MEDIA_SMOKE_TOKEN, 'scope_private_session_value')
writeFileSync(value('--receipt'), '{"passed":true,"request_deleted":true}\n')
appendFileSync(process.env.TRACE_PATH, 'media-smoke\n')
EOF
cat > "$test_root/media-capacity.mjs" <<'EOF'
import { appendFileSync, writeFileSync } from 'node:fs'
import assert from 'node:assert/strict'
const args = process.argv.slice(2)
const value = (name) => args[args.indexOf(name) + 1]
assert.equal(process.env.SCOPE_MEDIA_SMOKE_TOKEN, 'scope_private_session_value')
assert.equal(value('--small-uploads'), '4')
writeFileSync(value('--output'), '{"passed":true,"loaded":{"failed_requests":0}}\n')
appendFileSync(process.env.TRACE_PATH, 'media-capacity\n')
EOF

capacity_dir="$test_root/capacity-smoke"
capacity_token="$capacity_dir/exchange-token"
mkdir -m 0700 "$capacity_dir"
printf '%s\n' 'scope_otc_capacity_exchange' > "$capacity_token"
chmod 0600 "$capacity_token"
printf 'photo' > "$test_root/photo.png"
printf 'video' > "$test_root/video.mp4"
FAKE_SCOPE_SESSION_TOKEN='scope_private_session_value' \
  SCOPE_API_URL='https://api-staging.example.test' \
  SCOPE_GIT_ROUTER_URL='https://router-staging.example.test' \
  SCOPE_CLI_BINARY="$fake_bin/scope" \
  SCOPE_EXCHANGE_TOKEN_PATH="$capacity_token" \
  SCOPE_GIT_SMOKE_DIR="$capacity_dir" \
  SCOPE_MEDIA_GATEWAY_URL='https://media-staging.example.test' \
  SCOPE_MEDIA_SMOKE_SCRIPT="$test_root/media-smoke.mjs" \
  SCOPE_MEDIA_SMOKE_PNG="$test_root/photo.png" \
  SCOPE_MEDIA_SMOKE_MP4="$test_root/video.mp4" \
  SCOPE_MEDIA_SMOKE_RECEIPT="$test_root/media-receipt.json" \
  SCOPE_MEDIA_SMOKE_SOURCE_SHA='1111111111111111111111111111111111111111' \
  SCOPE_MEDIA_CAPACITY_SCRIPT="$test_root/media-capacity.mjs" \
  SCOPE_MEDIA_CAPACITY_VIDEO="$test_root/video.mp4" \
  SCOPE_MEDIA_CAPACITY_RECEIPT="$test_root/capacity-receipt.json" \
  GITHUB_SHA='test-sha' \
  bash "$repo_root/.github/scripts/staging-git-smoke.sh" > "$test_root/capacity-output" 2>&1
test ! -e "$capacity_dir"
grep -Fxq 'media-smoke' "$trace_path"
grep -Fxq 'media-capacity' "$trace_path"
if grep -Fq 'scope_private_session_value' "$test_root/capacity-output" "$trace_path"; then
  echo "staging capacity proof exposed its private session" >&2
  exit 1
fi

redirect_dir="$test_root/redirect-smoke"
redirect_token="$redirect_dir/exchange-token"
mkdir -m 0700 "$redirect_dir"
printf '%s\n' 'scope_otc_second_private_value' > "$redirect_token"
chmod 0600 "$redirect_token"
if FAKE_ROUTER_DIRECT=0 \
  SCOPE_API_URL='https://api-staging.example.test' \
  SCOPE_GIT_ROUTER_URL='https://router-staging.example.test' \
  SCOPE_CLI_BINARY="$fake_bin/scope" \
  SCOPE_EXCHANGE_TOKEN_PATH="$redirect_token" \
  SCOPE_GIT_SMOKE_DIR="$redirect_dir" \
  bash "$repo_root/.github/scripts/staging-git-smoke.sh" > "$test_root/redirect-output" 2>&1; then
  echo "staging Git smoke accepted a router redirect" >&2
  exit 1
fi
grep -Fq 'did not serve Git discovery directly' "$test_root/redirect-output"
test ! -e "$redirect_dir"

browser_line="$(grep -n -- '- name: Run browser smoke against staging' "$repo_root/.github/workflows/scope-railway-staging.yml" | cut -d: -f1)"
git_line="$(grep -n -- '- name: Run Git router smoke against staging' "$repo_root/.github/workflows/scope-railway-staging.yml" | cut -d: -f1)"
test "$browser_line" -lt "$git_line"
