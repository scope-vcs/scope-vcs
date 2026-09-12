import type { RepoParams, RequestParams } from './types'
import { buildApiPath } from './types.generated'

export function repoRoute(template: string, params: RepoParams) {
  return buildApiPath(template, { owner: params.owner, repo: params.repo })
}

export function requestRoute(template: string, params: RequestParams) {
  return buildApiPath(template, {
    owner: params.owner,
    repo: params.repo,
    request_id: params.request_id,
  })
}
