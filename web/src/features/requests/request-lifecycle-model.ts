import type { RequestSummaryResponse } from '@/api/types.generated'

export function canMergeRequest(request: RequestSummaryResponse) {
  return request.permissions.can_merge && request.mergeability.status === 'Ready'
}

// Decides whether the lifecycle action bar renders at all, so the page reserves
// space for it only when a button will actually appear.
export function hasRequestLifecycleActions(request: RequestSummaryResponse) {
  const { permissions } = request
  return permissions.can_submit || canMergeRequest(request) || permissions.can_close
}
