#!/usr/bin/env bash
set -euo pipefail

postgres_version=18.6
postgres_major="${postgres_version%%.*}"

key_sha256=0144068502a1eddd2a0280ede10ef607d1ec592ce819940991203941564e8e76
key=/etc/apt/keyrings/postgresql.asc

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --silent --show-error --retry 3 --retry-all-errors \
  https://www.postgresql.org/media/keys/ACCC4CF8.asc --output "$work/postgresql.asc"
printf '%s  %s\n' "$key_sha256" "$work/postgresql.asc" | sha256sum --check --strict
install -D -m 0644 "$work/postgresql.asc" "$key"

# shellcheck source=/dev/null
. /etc/os-release
printf 'deb [signed-by=%s] https://apt.postgresql.org/pub/repos/apt %s-pgdg main %s\n' "$key" "$VERSION_CODENAME" "$postgres_major" \
  > /etc/apt/sources.list.d/postgresql.list
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install --yes --no-install-recommends postgresql-common
sed -i -E 's/^#?[[:space:]]*create_main_cluster[[:space:]]*=.*/create_main_cluster = false/' \
  /etc/postgresql-common/createcluster.conf
package_version="$(apt-cache madison "postgresql-$postgres_major" | awk -v prefix="$postgres_version-" 'index($3, prefix) == 1 {print $3; exit}')"
if [[ -z "$package_version" ]]; then
  echo "PostgreSQL $postgres_version is not available for $VERSION_CODENAME" >&2
  exit 1
fi
apt-get install --yes --no-install-recommends "postgresql-$postgres_major=$package_version"
"/usr/lib/postgresql/$postgres_major/bin/initdb" --version
