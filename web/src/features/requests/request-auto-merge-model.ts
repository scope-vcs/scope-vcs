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

export function autoMergeIntentTitle(status: RequestAutoMergeIntentStatus) {
  switch (status) {
    case 'Active':
      return 'Will merge when checks pass'
    case 'Cancelled':
      return 'Auto-merge canceled'
    case 'Stopped':
      return 'Auto-merge stopped'
    case 'Fulfilled':
      return 'Merged automatically'
  }
  status satisfies never
  return 'Auto-merge'
}

export function autoMergeStopReasonText(reason: RequestAutoMergeStopReason) {
  switch (reason) {
    case 'RequestChanged':
      return 'the request changed'
    case 'RequestClosed':
      return 'the request closed'
    case 'AccessRevoked':
      return 'the authorizer no longer has access'
    case 'ChecksFailed':
      return 'checks failed'
    case 'ChecksConfigurationError':
      return 'the checks configuration is invalid'
    case 'MergeConflict':
      return 'the request conflicts with main'
    case 'RequestBranchMissing':
      return 'the request branch is missing'
  }
  reason satisfies never
  return 'auto-merge stopped'
}
