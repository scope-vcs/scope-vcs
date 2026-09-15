#!/usr/bin/env bash
set -euo pipefail

if (( $# != 3 )) || [[ "$1" != docker && "$1" != remote ]]; then
  echo "usage: $0 <docker|remote> <executable image> <JSON report>" >&2
  exit 2
fi
readonly image_source="$1" image="$2" report="$3"
if [[ "$image_source" == remote && ! "$image" =~ @sha256:[0-9a-f]{64}$ ]]; then
  echo 'Remote scans require an immutable executable image digest' >&2
  exit 2
fi

readonly version=0.74.0
readonly archive_sha256=2ae6fe3ee734b7fdf11335663e18c75ea12dccc76062f09f164a3b0f8be4371a
work_dir="$(mktemp -d)"
trap 'rm -rf -- "$work_dir"' EXIT
curl --fail --location --silent --show-error --retry 3 \
  "https://github.com/aquasecurity/trivy/releases/download/v${version}/trivy_${version}_Linux-64bit.tar.gz" \
  --output "$work_dir/trivy.tar.gz"
echo "$archive_sha256  $work_dir/trivy.tar.gz" | sha256sum --check --strict
tar --extract --gzip --file "$work_dir/trivy.tar.gz" --directory "$work_dir" trivy

# Scan all OS and language findings with current vendor data. Repository-local
# Trivy config/ignore files must not silently weaken the promotion policy.
"$work_dir/trivy" image --config /dev/null --ignorefile /dev/null \
  --image-src "$image_source" --platform linux/amd64 \
  --scanners vuln --pkg-types os,library --severity UNKNOWN,LOW,MEDIUM,HIGH,CRITICAL \
  --ignore-unfixed=false --skip-db-update=false --skip-java-db-update=false \
  --timeout 15m --exit-code 0 --format json --output "$report" "$image"
node "$(dirname "$0")/scan-report.mjs" "$report"
