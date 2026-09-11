import type { RequestSummary } from '@/api/types'

export function canMergeRequest(request: RequestSummary) {
  return request.permissions.can_merge && request.mergeability.status === 'Ready'
}

// Decides whether the lifecycle action bar renders at all, so the page reserves
// space for it only when a button will actually appear.
export function hasRequestLifecycleActions(request: RequestSummary) {
  const { permissions } = request
  return permissions.can_submit || canMergeRequest(request) || permissions.can_close
}
