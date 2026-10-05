#!/usr/bin/env bash
set -euo pipefail

key_sha256=0144068502a1eddd2a0280ede10ef607d1ec592ce819940991203941564e8e76
key=/etc/apt/keyrings/postgresql.asc

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  https://www.postgresql.org/media/keys/ACCC4CF8.asc --output "$work/postgresql.asc"
printf '%s  %s\n' "$key_sha256" "$work/postgresql.asc" | sha256sum --check --strict
install -D -m 0644 "$work/postgresql.asc" "$key"

# shellcheck source=/dev/null
. /etc/os-release
printf 'deb [signed-by=%s] https://apt.postgresql.org/pub/repos/apt %s-pgdg main 18\n' "$key" "$VERSION_CODENAME" \
  > /etc/apt/sources.list.d/postgresql.list
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install --yes --no-install-recommends postgresql-common
sed -i -E 's/^#?[[:space:]]*create_main_cluster[[:space:]]*=.*/create_main_cluster = false/' \
  /etc/postgresql-common/createcluster.conf
apt-get install --yes --no-install-recommends postgresql-18
/usr/lib/postgresql/18/bin/initdb --version
