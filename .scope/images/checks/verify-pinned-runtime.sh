#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

dockerfile=.scope/images/checks/Dockerfile
docker_arg() {
  sed -n "s/^ARG $1=//p" "$dockerfile"
}

expect_version() {
  local label="$1" expected="$2" actual
  shift 2
  actual="$("$@")" || {
    echo "$label is unavailable in the pinned checks image" >&2
    return 1
  }
  if [[ "$actual" != "$expected" ]]; then
    echo "$label mismatch in the pinned checks image: expected '$expected', found '$actual'" >&2
    return 1
  fi
}

rustc_version() {
  rustc --version | awk '{print $2}'
}

chromium_version() {
  "${PLAYWRIGHT_BROWSERS_PATH:?}/chromium-${chromium_revision}/chrome-linux64/chrome" --version \
    | sed 's/[[:space:]]*$//'
}

rust="$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)"
git_version="$(jq -er '.git.version' dev/tool-versions.json)"
node_version="$(jq -er '.node_version' cli/distribution/targets.json)"
npm_version="$(docker_arg NPM_VERSION)"
pnpm_version="$(docker_arg PNPM_VERSION)"
playwright_version="$(docker_arg PLAYWRIGHT_VERSION)"
chromium_version="$(docker_arg PLAYWRIGHT_CHROMIUM_VERSION)"
chromium_revision="$(docker_arg PLAYWRIGHT_CHROMIUM_REVISION)"

for value in "$rust" "$git_version" "$node_version" "$npm_version" "$pnpm_version" "$playwright_version" "$chromium_version" "$chromium_revision"; do
  [[ -n "$value" ]] || { echo 'Cannot read a required checks-image version pin' >&2; exit 1; }
done

expected_toolchain="${rust}-x86_64-unknown-linux-gnu"
expect_version RUSTUP_TOOLCHAIN "$expected_toolchain" printenv RUSTUP_TOOLCHAIN
if ! rustup toolchain list | awk '{print $1}' | grep -Fxq "$expected_toolchain"; then
  echo "Rust $expected_toolchain is absent from the pinned checks image" >&2
  exit 1
fi
for component in rustfmt clippy; do
  if ! rustup component list --installed --toolchain "$expected_toolchain" | grep -q "^${component}-"; then
    echo "$component is absent from Rust $expected_toolchain in the pinned checks image" >&2
    exit 1
  fi
done

expect_version Rust "$rust" rustc_version
expect_version Git "git version $git_version" git --version
expect_version Node "v$node_version" node --version
expect_version npm "$npm_version" npm --version
expect_version pnpm "$pnpm_version" pnpm --version
expect_version Playwright "Version $playwright_version" playwright --version
expect_version Chromium "Google Chrome for Testing $chromium_version" chromium_version

echo "Pinned checks image has Rust $rust, Git $git_version, Node $node_version, and Playwright $playwright_version."
