import type {
  RequestAutoMergeIntentStatus,
  RequestAutoMergeStopReason,
} from '../../api/types.generated'

export function autoMergeAuthorizer(
  actor: { handle: string; id: string },
  viewerId: string,
) {
  return actor.id === viewerId ? 'Authorized by you' : `Authorized by ${actor.handle}`
}

const titles: Record<RequestAutoMergeIntentStatus, string> = {
  Active: 'Will merge when checks pass',
  Cancelled: 'Auto-merge canceled',
  Stopped: 'Auto-merge stopped',
  Fulfilled: 'Merged automatically',
}

const reasons: Record<RequestAutoMergeStopReason, string> = {
  RequestChanged: 'the request changed',
  RequestClosed: 'the request closed',
  AccessRevoked: 'the authorizer no longer has access',
  ChecksFailed: 'checks failed',
  ChecksConfigurationError: 'the checks configuration is invalid',
  MergeConflict: 'the request conflicts with main',
  RequestBranchMissing: 'the request branch is missing',
}

export const autoMergeIntentTitle = (status: RequestAutoMergeIntentStatus) => titles[status] ?? 'Auto-merge'
export const autoMergeStopReasonText = (reason: RequestAutoMergeStopReason) => reasons[reason] ?? 'auto-merge stopped'
