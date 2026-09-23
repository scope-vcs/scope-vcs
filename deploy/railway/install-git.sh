#!/usr/bin/env bash
set -euo pipefail

version="${1:?Git version is required}"
source_sha256="${2:?Git source SHA-256 is required}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$source_sha256" =~ ^[0-9a-f]{64}$ ]]

apt-get update
apt-get install --yes --no-install-recommends \
  build-essential ca-certificates curl libcurl4-openssl-dev libexpat1-dev \
  libssl-dev xz-utils zlib1g-dev
archive="/tmp/git-${version}.tar.xz"
curl --fail --location --silent --show-error --retry 3 \
  "https://www.kernel.org/pub/software/scm/git/git-${version}.tar.xz" \
  --output "$archive"
printf '%s  %s\n' "$source_sha256" "$archive" | sha256sum --check --strict
tar --extract --xz --file "$archive" --directory /tmp
make -C "/tmp/git-${version}" -j 4 prefix=/opt/git \
  NO_GETTEXT=YesPlease NO_TCLTK=YesPlease NO_RUST=YesPlease all
make -C "/tmp/git-${version}" prefix=/opt/git \
  NO_GETTEXT=YesPlease NO_TCLTK=YesPlease NO_RUST=YesPlease install
test "$(/opt/git/bin/git --version)" = "git version ${version}"
doc_dir=/opt/git/share/doc/git
mkdir -p "$doc_dir"
install -m 0644 "/tmp/git-${version}/COPYING" "$doc_dir/COPYING"
install -m 0644 "$archive" "$doc_dir/git-${version}.tar.xz"
install -m 0644 /tmp/install-git.sh "$doc_dir/install-git.sh"
printf 'Source: https://www.kernel.org/pub/software/scm/git/git-%s.tar.xz\nSHA-256: %s\n' \
  "$version" "$source_sha256" > "$doc_dir/SOURCE.txt"
rm -rf "/tmp/git-${version}" "$archive" /var/lib/apt/lists/*
