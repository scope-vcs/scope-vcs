#!/usr/bin/env bash
set -euo pipefail

body="${1:?usage: preview-comment.sh <body>}"
repository="${REPOSITORY:?REPOSITORY is required}"
pull_request="${PULL_REQUEST:?PULL_REQUEST is required}"
[[ "$pull_request" =~ ^[1-9][0-9]*$ ]]
comment="$(gh api --paginate "repos/$repository/issues/$pull_request/comments" \
  --jq '.[] | select(.user.login == "github-actions[bot]" and (.body | startswith("**Preview environment**"))) | .id' | head -n 1)"
if [[ -n "$comment" ]]; then
  gh api --method PATCH "repos/$repository/issues/comments/$comment" -f body="$body" >/dev/null
else
  gh api "repos/$repository/issues/$pull_request/comments" -f body="$body" >/dev/null
fi
