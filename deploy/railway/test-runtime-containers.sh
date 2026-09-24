#!/usr/bin/env bash
set -euo pipefail

# Build the release Dockerfiles with small executable fixtures. This checks the
# actual image user and filesystem permissions without production credentials.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
context="$(mktemp -d)"
image_prefix="scope-runtime-permissions-$$"
cleanup() {
  rm -rf "$context"
  docker image rm "$image_prefix-prebuilt" "$image_prefix-worker" "$image_prefix-web" "$image_prefix-maintenance" >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "$context/bin" "$context/.output/server"
cat > "$context/bin/scope-worker" <<'SH'
#!/bin/sh
set -eu
test "$(id -u)" = 65532
test "$(id -g)" = 65532
test -w "$HOME"
test -w "$XDG_CACHE_HOME"
test ! -w /app
test ! -w /app/bin/scope-worker
test "$(git --version)" = "git version ${SCOPE_TEST_GIT_VERSION}"
test -r /opt/git/share/doc/git/COPYING
test -r /opt/git/share/doc/git/install-git.sh
test -r /opt/git/share/doc/git/SOURCE.txt
test "$(sha256sum "/opt/git/share/doc/git/git-${SCOPE_TEST_GIT_VERSION}.tar.xz" | cut -d ' ' -f 1)" = "$SCOPE_TEST_GIT_SHA256"
mkdir -p .scope/git-segments .scope/dependency-checks
printf 'write probe' > .scope/git-segments/probe
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
git init --bare --template= "$scratch/repository"
git --git-dir="$scratch/repository" hash-object -w --stdin < .scope/git-segments/probe
if [ -n "${SCOPE_DEPENDENCY_ANALYZER_PATH:-}" ]; then
  test ! -w "$SCOPE_DEPENDENCY_ANALYZER_PATH"
  mkdir -p .scope/dependency-checks/source
  printf 'export const value = 1;\n' > .scope/dependency-checks/source/value.js
  printf 'import { value } from "./value.js";\n' > .scope/dependency-checks/source/index.js
  node "$SCOPE_DEPENDENCY_ANALYZER_PATH" .scope/dependency-checks/source > "$scratch/analysis.json"
  node -e 'const fs = require("node:fs"); JSON.parse(fs.readFileSync(process.argv[1], "utf8"))' "$scratch/analysis.json"
fi
SH
chmod 755 "$context/bin/scope-worker"
cp -R "$repo_root/dependency-analyzer" "$context/dependency-analyzer"
cp "$repo_root/legal/third-party-dependency-analyzer.txt" "$context/dependency-analyzer/"
cp "$repo_root/deploy/railway/install-git.sh" "$context/install-git.sh"
git_version="$(jq -er '.git.version' "$repo_root/dev/tool-versions.json")"
git_source_sha256="$(jq -er '.git.sourceSha256' "$repo_root/dev/tool-versions.json")"
# The dependency stage must supply node_modules from the reviewed lockfile.
rm -rf "$context/dependency-analyzer/node_modules"

docker build -f "$repo_root/deploy/railway/prebuilt.Dockerfile" \
  --build-arg INSTALL_GIT=1 --build-arg BINARY=scope-worker \
  --build-arg "GIT_VERSION=$git_version" --build-arg "GIT_SOURCE_SHA256=$git_source_sha256" \
  -t "$image_prefix-prebuilt" "$context"
docker run --rm --network none -e "SCOPE_TEST_GIT_VERSION=$git_version" -e "SCOPE_TEST_GIT_SHA256=$git_source_sha256" "$image_prefix-prebuilt"

docker build -f "$repo_root/deploy/railway/worker.Dockerfile" \
  --build-arg "GIT_VERSION=$git_version" --build-arg "GIT_SOURCE_SHA256=$git_source_sha256" \
  -t "$image_prefix-worker" "$context"
docker run --rm --network none -e "SCOPE_TEST_GIT_VERSION=$git_version" -e "SCOPE_TEST_GIT_SHA256=$git_source_sha256" "$image_prefix-worker"

cp "$context/bin/scope-worker" "$context/bin/scope-maintenance"
cat >> "$context/bin/scope-maintenance" <<'SH'
test "$1" = serve
test ! -w /app/bin/scope-maintenance
pg_dump --version | grep -E 'PostgreSQL\) 18\.'
pg_restore --version | grep -E 'PostgreSQL\) 18\.'
psql --version | grep -E 'PostgreSQL\) 18\.'
printf 'checksum probe' | sha256sum
SH
docker build -f "$repo_root/deploy/railway/maintenance.Dockerfile" \
  --build-arg "GIT_VERSION=$git_version" --build-arg "GIT_SOURCE_SHA256=$git_source_sha256" \
  -t "$image_prefix-maintenance" "$context"
docker run --rm --network none -e "SCOPE_TEST_GIT_VERSION=$git_version" -e "SCOPE_TEST_GIT_SHA256=$git_source_sha256" "$image_prefix-maintenance"

cat > "$context/.output/server/index.mjs" <<'JS'
import assert from 'node:assert/strict';
import { accessSync, constants, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

assert.equal(process.getuid(), 1000);
assert.equal(process.getgid(), 1000);
accessSync(process.env.HOME, constants.W_OK);
assert.throws(() => accessSync('/app', constants.W_OK));
assert.throws(() => accessSync('/app/.output/server/index.mjs', constants.W_OK));
assert.equal(readFileSync('/app/.scope-deployment-sha', 'utf8').trim(), 'permission-test');
const scratch = mkdtempSync(join(tmpdir(), 'scope-web-'));
writeFileSync(join(scratch, 'probe'), 'ok');
rmSync(scratch, { recursive: true });
JS
printf 'permission-test\n' > "$context/.scope-deployment-sha"
docker build -f "$repo_root/deploy/railway/web.Dockerfile" \
  -t "$image_prefix-web" "$context"
docker run --rm --network none "$image_prefix-web"
printf 'Runtime container permission checks passed.\n'
