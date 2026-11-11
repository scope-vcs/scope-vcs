#!/usr/bin/env bash
set -euo pipefail

postgres_version=18.6
postgres_major="${postgres_version%%.*}"

key_sha256=0144068502a1eddd2a0280ede10ef607d1ec592ce819940991203941564e8e76
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 0755 "$work"
mkdir "$work/lists"
key="$work/postgresql.asc"

vendored_key="$(dirname "$0")/install-test-postgres.asc"
printf '%s  %s\n' "$key_sha256" "$vendored_key" | sha256sum --check --strict
install -D -m 0644 "$vendored_key" "$key"

# shellcheck source=/dev/null
. /etc/os-release
printf 'deb [signed-by=%s] https://apt.postgresql.org/pub/repos/apt %s-pgdg main %s\n' "$key" "$VERSION_CODENAME" "$postgres_major" \
  > "$work/sources.list"
printf 'deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu %s main\n' \
  "$VERSION_CODENAME" "$VERSION_CODENAME-updates" >> "$work/sources.list"
printf 'deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://security.ubuntu.com/ubuntu %s-security main\n' \
  "$VERSION_CODENAME" >> "$work/sources.list"
apt_options=(
  -o "Dir::Etc::sourcelist=$work/sources.list"
  -o Dir::Etc::sourceparts=-
  -o "Dir::State::lists=$work/lists"
)
export DEBIAN_FRONTEND=noninteractive
apt-get "${apt_options[@]}" -o APT::Update::Error-Mode=any update
apt-get "${apt_options[@]}" install --yes --no-install-recommends postgresql-common locales
localedef -i en_US -f UTF-8 en_US.utf8
sed -i -E 's/^#?[[:space:]]*create_main_cluster[[:space:]]*=.*/create_main_cluster = false/' \
  /etc/postgresql-common/createcluster.conf
package_version="$(apt-cache "${apt_options[@]}" madison "postgresql-$postgres_major" | awk -v prefix="$postgres_version-" 'index($3, prefix) == 1 {print $3; exit}')"
if [[ -z "$package_version" ]]; then
  echo "PostgreSQL $postgres_version is not available for $VERSION_CODENAME" >&2
  exit 1
fi
apt-get "${apt_options[@]}" install --yes --no-install-recommends "postgresql-$postgres_major=$package_version"
"/usr/lib/postgresql/$postgres_major/bin/initdb" --version
