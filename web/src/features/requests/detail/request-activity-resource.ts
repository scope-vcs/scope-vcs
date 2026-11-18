import { affectsRequestKey, createRepoResource, repoResourceKey } from '../../repo-detail/repo-resource'
import type { RequestActivityPage } from '../discussion/request-discussion-types'

export const requestActivityResource = createRepoResource<RequestActivityPage>({
  invalidatedBy: ['requestTimeline'],
  affectedBy: affectsRequestKey,
  maxEntries: 16,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestActivityIdentity(scope: string, requestId: string) {
  return repoResourceKey(scope, requestId)
}
