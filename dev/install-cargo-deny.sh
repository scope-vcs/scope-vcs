#!/usr/bin/env bash
set -euo pipefail

# Installs the cargo-deny release pinned in dev/tool-versions.json after
# verifying the published Linux x86_64 musl archive against the pinned SHA-256.
install_dir="${1:-$HOME/.cargo/bin}"
versions="$(dirname "${BASH_SOURCE[0]}")/tool-versions.json"
version="$(jq -er '.cargoDeny.version' "$versions")"
sha256="$(jq -er '.cargoDeny.linuxMuslSha256' "$versions")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$sha256" =~ ^[0-9a-f]{64}$ ]]

name="cargo-deny-${version}-x86_64-unknown-linux-musl"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  "https://github.com/EmbarkStudios/cargo-deny/releases/download/${version}/${name}.tar.gz" \
  --output "$work/${name}.tar.gz"
printf '%s  %s\n' "$sha256" "$work/${name}.tar.gz" | sha256sum --check --strict
tar --extract --gzip --file "$work/${name}.tar.gz" --directory "$work"
mkdir -p "$install_dir"
install -m 0755 "$work/${name}/cargo-deny" "$install_dir/cargo-deny"
test "$("$install_dir/cargo-deny" --version)" = "cargo-deny ${version}"
