import type {
  RequestAutoMergeResponse,
  RequestChecksResponse,
  RequestMergeabilityStatus,
  RequestSummaryResponse,
} from '@/api/types.generated'

// Mergeability that describes a request which is no longer, or not yet, open.
const NOT_OPEN = new Set<RequestMergeabilityStatus>(['Draft', 'Closed', 'Merged'])

/**
 * Run changes refresh a request's checks but not its summary. While the request
 * stays open on the same head, the checks carry its current mergeability. Checks
 * loaded before the request opened, such as while it was a draft, do not.
 */
export function withCurrentMergeability(
  request: RequestSummaryResponse,
  checks: RequestChecksResponse | null,
): RequestSummaryResponse {
  const current = checks?.mergeability
  return request.state === 'Open' && current?.request_head_oid === request.head_oid &&
    !NOT_OPEN.has(current.status)
    ? { ...request, mergeability: current }
    : request
}

export function canMergeRequest(request: RequestSummaryResponse) {
  return request.permissions.can_merge && request.mergeability.status === 'Ready'
}

/**
 * A maintainer whose merge waits only on checks still sees the merge, disabled.
 * The mergeability badge beside it says why.
 */
export function checksHoldRequestMerge(request: RequestSummaryResponse) {
  return request.permissions.can_merge &&
    request.mergeability.status.startsWith('Checks')
}

// Decides whether the lifecycle action bar renders at all, so the page reserves
// space for it only when a button will actually appear.
export function hasRequestLifecycleActions(request: RequestSummaryResponse) {
  return request.permissions.can_submit || canMergeRequest(request) ||
    checksHoldRequestMerge(request)
}

export function hasRequestAutoMergeActions(
  status: Pick<RequestAutoMergeResponse, 'can_enable' | 'intent'> | null,
  dialogOpen = false,
) {
  return dialogOpen || (
    status !== null && (status.can_enable || status.intent?.status === 'Active')
  )
}
