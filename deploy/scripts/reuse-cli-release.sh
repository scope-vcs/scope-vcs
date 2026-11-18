#!/usr/bin/env bash
set -euo pipefail

tag="${1:?usage: reuse-cli-release.sh <tag> <manifest-output>}"
output="${2:?usage: reuse-cli-release.sh <tag> <manifest-output>}"
repository="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
source_sha="${GITHUB_SHA:?GITHUB_SHA is required}"

refuse() {
  echo "CLI release $tag $1; delete the release before publishing it again." >&2
  exit 1
}

if ! release="$(gh release view "$tag" --repo "$repository" --json assets,isDraft 2>/dev/null)"; then
  echo "exists=false"
  exit 0
fi

jq -e '.isDraft == false' <<< "$release" >/dev/null || refuse "is a draft that nobody can download"
assets="$(jq -r '.assets[].name' <<< "$release")"
while IFS= read -r expected; do
  grep -Fxq -- "$expected" <<< "$assets" || refuse "exists but lacks $expected"
done < <(jq -r '.targets[].artifact, "cli-release.json"' cli/distribution/targets.json)

directory="$(mktemp -d)"
trap 'rm -rf "$directory"' EXIT
gh release download "$tag" --repo "$repository" --dir "$directory"
gh attestation verify "$directory/cli-release.json" --repo "$repository" >/dev/null
jq -e --arg tag "$tag" --arg sha "$source_sha" --slurpfile targets cli/distribution/targets.json '
  .tag == $tag and .sourceSha == $sha
  and ((.artifacts | keys) == ([$targets[0].targets[].artifact] | sort))
' "$directory/cli-release.json" >/dev/null || refuse "has a manifest that does not describe this revision"
while read -r artifact attested; do
  [[ "$(sha256sum "$directory/$artifact" | cut -d ' ' -f 1)" == "$attested" ]] \
    || refuse "serves $artifact with a digest its attested manifest does not record"
done < <(jq -r '.artifacts | to_entries[] | "\(.key) \(.value)"' "$directory/cli-release.json")
mkdir -p "$(dirname "$output")"
cp "$directory/cli-release.json" "$output"
echo "exists=true"
