import type {
  CreateRequestAttachmentMediaGrantResponse,
  RequestAttachmentLimitsResponse,
  RequestAttachmentResponse,
} from '@/api/types.generated'
import { affectsRequestKey, createRepoResource, repoResourceKey, repoResourceKeyParts } from '../../repo-detail/repo-resource'
import { onViewerChange } from '../../../lib/viewer-state'
import { repoResourceScopeOwner } from '../../repo-detail/repo-resource-scope'

export type RequestAttachmentResourceValue = {
  attachments: RequestAttachmentResponse[]
  limits: RequestAttachmentLimitsResponse
}

export const requestAttachmentResource = createRepoResource<RequestAttachmentResourceValue>({
  invalidatedBy: ['requestTimeline', 'requestAttachments'],
  affectedBy: affectsRequestKey,
  maxEntries: 16,
  maxWeight: 8 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export const requestAttachmentMediaGrantResource = createRepoResource<CreateRequestAttachmentMediaGrantResponse>({
  invalidatedBy: [],
  maxEntries: 64,
})

const MAX_TRACKED_OWNERS = 64
const activeAccessScopes = new Map<string, string>()
onViewerChange(() => activeAccessScopes.clear())

export function activateRequestAttachmentResourceScope(accessScope: string) {
  const owner = repoResourceScopeOwner(accessScope)
  if (!owner) return
  const previous = activeAccessScopes.get(owner)
  if (previous && previous !== accessScope) {
    const inPreviousScope = (identity: string) => repoResourceKeyParts(previous, identity) !== null
    requestAttachmentResource.removeMatching(inPreviousScope)
    requestAttachmentMediaGrantResource.removeMatching(inPreviousScope)
  }
  activeAccessScopes.delete(owner)
  activeAccessScopes.set(owner, accessScope)
  if (activeAccessScopes.size > MAX_TRACKED_OWNERS) {
    const oldest = activeAccessScopes.keys().next().value
    if (oldest) activeAccessScopes.delete(oldest)
  }
}

export function requestAttachmentResourceIdentity(
  accessScope: string,
  requestId: string,
) {
  return repoResourceKey(accessScope, requestId)
}

export function refreshRequestAttachments(accessScope: string, requestId: string) {
  requestAttachmentResource.invalidate(
    requestAttachmentResourceIdentity(accessScope, requestId),
  )
}
