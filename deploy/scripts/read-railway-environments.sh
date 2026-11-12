#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/railway-graphql.sh"
project_id="$(jq -er '.railway.projectId' .github/deployment-services.json)"
query='query($id:String!){project(id:$id){id environments(first:100){pageInfo{hasNextPage} edges{node{id name createdAt isEphemeral sourceEnvironment{id}}}}}}'
railway_graphql read "$query" "$(jq -cn --arg id "$project_id" '{id:$id}')"
