import type {
  RequestAutoMergeResponse,
  RequestSummaryResponse,
} from '@/api/types.generated'
import type { RepoViews } from '../../api/repo-views'

export function canMergeRequest(request: RequestSummaryResponse) {
  return request.permissions.can_merge && request.mergeability.status === 'Ready'
}

export function requestMergeHeld(request: RequestSummaryResponse) {
  return request.permissions.can_merge &&
    (request.mergeability.status.startsWith('Checks') ||
      ['WaitingForParent', 'ParentRewritten', 'ParentClosed', 'AncestorBlocked', 'IntegrationConflict', 'NeedsMainUpdate', 'ReviewsPending', 'ReviewFindingsOpen'].includes(request.mergeability.status))
}

export function hasRequestLifecycleActions(request: RequestSummaryResponse) {
  return request.permissions.can_submit || canMergeRequest(request) ||
    requestMergeHeld(request)
}

export function hasRequestAutoMergeActions(
  status: Pick<RequestAutoMergeResponse, 'can_enable' | 'intent'>,
  dialogOpen = false,
) {
  return dialogOpen || status.can_enable || status.intent?.status === 'Active'
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
