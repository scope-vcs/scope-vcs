#!/usr/bin/env bash
set -euo pipefail

install_dir="${1:-$HOME/.cargo/bin}"
versions="$(dirname "${BASH_SOURCE[0]}")/tool-versions.json"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$install_dir"

install_tool() {
  local tool="$1" key="$2" url="$3" binary="$4"
  local version sha256
  version="$(jq -er ".${key}.version" "$versions")"
  sha256="$(jq -er ".${key}.linuxMuslSha256" "$versions")"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
  [[ "$sha256" =~ ^[0-9a-f]{64}$ ]]
  url="${url//\{version\}/$version}"
  binary="${binary//\{version\}/$version}"
  mkdir "$work/$tool"
  curl --fail --location --silent --show-error --connect-timeout 10 --max-time 60 \
    --retry 7 --retry-all-errors --retry-max-time 180 \
    "$url" --output "$work/$tool/archive.tar.gz"
  printf '%s  %s\n' "$sha256" "$work/$tool/archive.tar.gz" | sha256sum --check --strict
  tar --extract --gzip --file "$work/$tool/archive.tar.gz" --directory "$work/$tool"
  install -m 0755 "$work/$tool/$binary" "$install_dir/$tool"
  "$install_dir/$tool" --version | grep -Fq "$version"
}

install_tool cargo-deny cargoDeny \
  "https://github.com/EmbarkStudios/cargo-deny/releases/download/{version}/cargo-deny-{version}-x86_64-unknown-linux-musl.tar.gz" \
  "cargo-deny-{version}-x86_64-unknown-linux-musl/cargo-deny"
install_tool cargo-shear cargoShear \
  "https://github.com/Boshen/cargo-shear/releases/download/v{version}/cargo-shear-x86_64-unknown-linux-musl.tar.gz" \
  "cargo-shear"
