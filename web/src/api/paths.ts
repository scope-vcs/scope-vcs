import type { RequestParams } from './types'
import { buildApiPath } from './types.generated'

export function requestRoute(template: string, params: RequestParams) {
  return buildApiPath(template, {
    owner: params.owner,
    repo: params.repo,
    request_id: params.request_id,
  })
}
