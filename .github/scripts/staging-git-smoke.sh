#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_API_URL:?SCOPE_API_URL is required}"
: "${SCOPE_GIT_ROUTER_URL:?SCOPE_GIT_ROUTER_URL is required}"
: "${SCOPE_CLI_BINARY:?SCOPE_CLI_BINARY is required}"
: "${SCOPE_EXCHANGE_TOKEN_PATH:?SCOPE_EXCHANGE_TOKEN_PATH is required}"
: "${SCOPE_GIT_SMOKE_DIR:?SCOPE_GIT_SMOKE_DIR is required}"

if [[ ! "$SCOPE_API_URL" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
  echo "SCOPE_API_URL must be an HTTPS origin without a path." >&2
  exit 2
fi
if [[ ! "$SCOPE_GIT_ROUTER_URL" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
  echo "SCOPE_GIT_ROUTER_URL must be an HTTPS origin without a path." >&2
  exit 2
fi
if [[ ! -x "$SCOPE_CLI_BINARY" ]]; then
  echo "SCOPE_CLI_BINARY must be executable." >&2
  exit 2
fi
if [[ "$SCOPE_GIT_SMOKE_DIR" != /* || "$SCOPE_GIT_SMOKE_DIR" == "/" ]]; then
  echo "SCOPE_GIT_SMOKE_DIR must be a specific absolute directory." >&2
  exit 2
fi
if [[ ! -d "$SCOPE_GIT_SMOKE_DIR" || -L "$SCOPE_GIT_SMOKE_DIR" ]]; then
  echo "SCOPE_GIT_SMOKE_DIR must be an existing physical directory." >&2
  exit 2
fi

smoke_dir="$(realpath "$SCOPE_GIT_SMOKE_DIR")"
token_path="$SCOPE_EXCHANGE_TOKEN_PATH"
if [[ "$token_path" != "$smoke_dir/exchange-token" ]]; then
  echo "The staging exchange token must be inside SCOPE_GIT_SMOKE_DIR." >&2
  exit 2
fi
# The workflow owns this root and removes its credentials in its always() cleanup.
# Each invocation owns only a fresh checkout directory beneath it.
private_directory() {
  [[ -d "$1" && ! -L "$1" && -O "$1" && "$(stat -c '%a' "$1")" == 700 ]]
}
private_file() {
  [[ -f "$1" && ! -L "$1" && -O "$1" && -s "$1" && "$(stat -c '%a' "$1")" == 600 ]]
}
if ! private_directory "$smoke_dir"; then
  echo "The staging smoke root must be owner-only with mode 0700." >&2
  exit 2
fi
umask 077
for directory in "$smoke_dir/config" "$smoke_dir/config/scope" "$smoke_dir/config/scope/sessions"; do
  if [[ ! -e "$directory" && ! -L "$directory" ]]; then mkdir "$directory"; fi
  if ! private_directory "$directory"; then
    echo "The staging CLI config must use private physical directories." >&2
    exit 2
  fi
done
export XDG_CONFIG_HOME="$smoke_dir/config"
# Match cli/src/auth.rs::session_storage_key exactly, including the API origin.
session_key="$(printf '%s' "$SCOPE_API_URL" | od -An -v -tx1 | tr -d ' \n')"
session_path="$XDG_CONFIG_HOME/scope/sessions/cli-session-$session_key"
if [[ -e "$session_path" || -L "$session_path" ]]; then
  if ! private_file "$session_path"; then
    echo "The staging CLI session must be a private regular file." >&2
    exit 2
  fi
elif ! private_file "$token_path"; then
  echo "The staging exchange token or scoped private CLI session is missing or unsafe." >&2
  exit 2
fi
scratch_dir="$(mktemp -d "$smoke_dir/invocation.XXXXXX")"
cleanup() {
  rm -rf -- "$scratch_dir"
}
trap cleanup EXIT

cli_binary="$(realpath "$SCOPE_CLI_BINARY")"
cli_directory="$(dirname "$cli_binary")"
export PATH="$cli_directory:$PATH"

repo="dev/update-demo"
public_url="$SCOPE_GIT_ROUTER_URL/git/public/$repo"
permissioned_url="$SCOPE_GIT_ROUTER_URL/git/permissioned/$repo"
public_checkout="$scratch_dir/public"
permissioned_checkout="$scratch_dir/permissioned"

discovery_headers="$scratch_dir/router-headers"
discovery_body="$scratch_dir/router-body"
status="$(curl --silent --show-error --max-redirs 0 \
  --output "$discovery_body" \
  --dump-header "$discovery_headers" \
  --write-out '%{http_code}' \
  "$public_url/info/refs?service=git-upload-pack")"
if [[ "$status" != "200" ]] || ! grep -Eiq '^x-scope-git-router:[[:space:]]*1[[:space:]]*$' "$discovery_headers"; then
  echo "The staging router domain did not serve Git discovery directly." >&2
  exit 1
fi
rm -f -- "$discovery_headers" "$discovery_body"

GIT_TERMINAL_PROMPT=0 git -c credential.helper= clone --quiet "$public_url" "$public_checkout"
test "$(git -C "$public_checkout" remote get-url origin)" = "$public_url"
test -f "$public_checkout/README.md"
test ! -e "$public_checkout/internal/notes.md"
previous_public_head="$(git -C "$public_checkout" rev-parse HEAD)"

if [[ ! -f "$session_path" ]]; then
  SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" login --exchange-file "$token_path"
  rm -f -- "$token_path"
  if ! private_file "$session_path"; then
    echo "Staging login did not create the scoped private CLI session." >&2
    exit 1
  fi
fi
if [[ -n "${SCOPE_MEDIA_SMOKE_SCRIPT:-}" ]]; then
  : "${SCOPE_MEDIA_GATEWAY_URL:?SCOPE_MEDIA_GATEWAY_URL is required for media smoke}"
  : "${SCOPE_MEDIA_SMOKE_PNG:?SCOPE_MEDIA_SMOKE_PNG is required for media smoke}"
  : "${SCOPE_MEDIA_SMOKE_MP4:?SCOPE_MEDIA_SMOKE_MP4 is required for media smoke}"
  : "${SCOPE_MEDIA_SMOKE_RECEIPT:?SCOPE_MEDIA_SMOKE_RECEIPT is required for media smoke}"
  : "${SCOPE_MEDIA_SMOKE_SOURCE_SHA:?SCOPE_MEDIA_SMOKE_SOURCE_SHA is required for media smoke}"
  [[ -f "$SCOPE_MEDIA_SMOKE_SCRIPT" && -f "$SCOPE_MEDIA_SMOKE_PNG" && -f "$SCOPE_MEDIA_SMOKE_MP4" ]]
  SCOPE_MEDIA_SMOKE_TOKEN="$(tr -d '\r\n' < "$session_path")" \
    node "$SCOPE_MEDIA_SMOKE_SCRIPT" \
      --api "$SCOPE_API_URL" \
      --media-origin "$SCOPE_MEDIA_GATEWAY_URL" \
      --repo dev/update-demo \
      --source-sha "$SCOPE_MEDIA_SMOKE_SOURCE_SHA" \
      --file "$SCOPE_MEDIA_SMOKE_PNG" \
      --file "$SCOPE_MEDIA_SMOKE_MP4" \
      --require-video \
      --receipt "$SCOPE_MEDIA_SMOKE_RECEIPT"
  if [[ -n "${SCOPE_MEDIA_CAPACITY_SCRIPT:-}" ]]; then
    : "${SCOPE_MEDIA_CAPACITY_VIDEO:?SCOPE_MEDIA_CAPACITY_VIDEO is required for capacity proof}"
    : "${SCOPE_MEDIA_CAPACITY_RECEIPT:?SCOPE_MEDIA_CAPACITY_RECEIPT is required for capacity proof}"
    [[ -f "$SCOPE_MEDIA_CAPACITY_SCRIPT" && -f "$SCOPE_MEDIA_CAPACITY_VIDEO" ]]
    SCOPE_MEDIA_SMOKE_TOKEN="$(tr -d '\r\n' < "$session_path")" \
      node "$SCOPE_MEDIA_CAPACITY_SCRIPT" \
        --api "$SCOPE_API_URL" \
        --media-origin "$SCOPE_MEDIA_GATEWAY_URL" \
        --repo dev/update-demo \
        --source-sha "$SCOPE_MEDIA_SMOKE_SOURCE_SHA" \
        --large-video "$SCOPE_MEDIA_CAPACITY_VIDEO" \
        --photo "$SCOPE_MEDIA_SMOKE_PNG" \
        --small-uploads 4 \
        --output "$SCOPE_MEDIA_CAPACITY_RECEIPT"
  fi
fi
SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" clone "$repo" "$permissioned_checkout"
test "$(git -C "$permissioned_checkout" remote get-url origin)" = "$permissioned_url"
test -f "$permissioned_checkout/internal/notes.md"
GIT_TERMINAL_PROMPT=0 git -C "$permissioned_checkout" fetch --quiet --prune origin

marker="Scope staging router smoke ${GITHUB_SHA:-manual} $(basename "$scratch_dir")"
printf '\n%s\n' "$marker" >> "$permissioned_checkout/README.md"
git -C "$permissioned_checkout" add README.md
git -C "$permissioned_checkout" \
  -c user.name='Scope staging smoke' \
  -c user.email='smoke@example.test' \
  commit --quiet -m 'Exercise the staging Git router'
expected_head="$(git -C "$permissioned_checkout" rev-parse HEAD)"
(
  cd "$permissioned_checkout"
  SCOPE_API_URL="$SCOPE_API_URL" "$cli_binary" push --main --no-review --remote origin
)

GIT_TERMINAL_PROMPT=0 git -C "$permissioned_checkout" fetch --quiet origin main
if [[ "$(git -C "$permissioned_checkout" rev-parse FETCH_HEAD)" != "$expected_head" ]]; then
  echo "The permissioned remote did not retain the accepted staging commit." >&2
  exit 1
fi

GIT_TERMINAL_PROMPT=0 git -C "$public_checkout" -c credential.helper= fetch --quiet origin main
public_head="$(git -C "$public_checkout" rev-parse FETCH_HEAD)"
if [[ "$public_head" == "$previous_public_head" ]]; then
  echo "The public projection did not advance after the staging push." >&2
  exit 1
fi
git -C "$public_checkout" show FETCH_HEAD:README.md | grep -Fqx "$marker"
if git -C "$public_checkout" cat-file -e FETCH_HEAD:internal/notes.md 2>/dev/null; then
  echo "The updated public projection exposed a private file." >&2
  exit 1
fi
echo "Staging Git router smoke passed for dev/update-demo."
