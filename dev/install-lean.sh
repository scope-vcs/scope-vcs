#!/usr/bin/env bash
set -euo pipefail

install_dir="${1:-$HOME/.local/share/lean}"
versions="$(dirname "${BASH_SOURCE[0]}")/tool-versions.json"
version="$(jq -er '.lean.version' "$versions")"
sha256="$(jq -er '.lean.linuxSha256' "$versions")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$sha256" =~ ^[0-9a-f]{64}$ ]]

if [[ -x "$install_dir/bin/lean" ]] && "$install_dir/bin/lean" --version | grep -q "version ${version},"; then
  exit 0
fi
name="lean-${version}-linux"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --silent --show-error --retry 4 --retry-all-errors --retry-delay 3 \
  "https://releases.lean-lang.org/lean4/v${version}/${name}.tar.zst" \
  --output "$work/${name}.tar.zst"
printf '%s  %s\n' "$sha256" "$work/${name}.tar.zst" | sha256sum --check --strict
tar --extract --zstd --file "$work/${name}.tar.zst" --directory "$work"
rm -rf "$install_dir"
mkdir -p "$(dirname "$install_dir")"
mv "$work/${name}" "$install_dir"
"$install_dir/bin/lean" --version | grep -q "version ${version},"
