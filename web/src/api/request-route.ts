import type { RequestParams } from './types'
import { buildApiPath } from './types.generated'

export function requestRoute(template: string, data: RequestParams) {
  return buildApiPath(template, {
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}
