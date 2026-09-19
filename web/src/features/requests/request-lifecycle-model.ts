import type {
  RequestAutoMergeResponse,
  RequestSummaryResponse,
} from '@/api/types.generated'

export function canMergeRequest(request: RequestSummaryResponse) {
  return request.permissions.can_merge && request.mergeability.status === 'Ready'
}

/**
 * A maintainer whose merge waits only on checks still sees the merge, disabled,
 * with the reason the server gave for holding it.
 */
export function checksHoldRequestMerge(request: RequestSummaryResponse) {
  return request.permissions.can_merge &&
    request.mergeability.status.startsWith('Checks')
}

// Decides whether the lifecycle action bar renders at all, so the page reserves
// space for it only when a button will actually appear.
export function hasRequestLifecycleActions(request: RequestSummaryResponse) {
  const { permissions } = request
  return permissions.can_submit || canMergeRequest(request) ||
    checksHoldRequestMerge(request) || permissions.can_close
}

export function hasRequestAutoMergeActions(
  status: Pick<RequestAutoMergeResponse, 'can_enable' | 'intent'> | null,
  dialogOpen = false,
) {
  return dialogOpen || (
    status !== null && (status.can_enable || status.intent !== null)
  )
}
