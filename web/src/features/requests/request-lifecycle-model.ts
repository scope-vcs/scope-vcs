import type {
  RequestAutoMergeResponse,
  RequestChecksResponse,
  RequestMergeabilityStatus,
  RequestSummaryResponse,
} from '@/api/types.generated'
import type { RepoViews } from '../../api/repo-views'

const NOT_OPEN = new Set<RequestMergeabilityStatus>(['Draft', 'Closed', 'Merged'])

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

export function checksHoldRequestMerge(request: RequestSummaryResponse) {
  return request.permissions.can_merge &&
    request.mergeability.status.startsWith('Checks')
}

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

export function requestSubmitsForReview(request: Pick<RequestSummaryResponse, 'author_role'>) {
  return request.author_role === 'Public'
}

export function requestShowsInvitees(
  request: Pick<RequestSummaryResponse, 'invitees' | 'view'>,
  views: Pick<RepoViews, 'anyone'>,
) {
  return request.view === views.anyone || request.invitees.length > 0
}
