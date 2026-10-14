import type { RequestAutoMergeStopReason } from '../../api/types.generated'

export function autoMergeAuthorizer(
  actor: { handle: string; id: string },
  viewerId: string,
) {
  return actor.id === viewerId ? 'Authorized by you' : `Authorized by ${actor.handle}`
}

const reasons: Record<RequestAutoMergeStopReason, string> = {
  RequestChanged: 'the request changed',
  RequestClosed: 'the request closed',
  AccessRevoked: 'the authorizer no longer has access',
  ChecksFailed: 'CI failed',
  ChecksConfigurationError: 'the CI configuration is invalid',
  MergeConflict: 'the request conflicts with main',
  RequestBranchMissing: 'the request branch is missing',
  ParentRewritten: 'the parent request was rewritten',
  ParentClosed: 'the parent request was closed',
  AncestorBlocked: 'an ancestor request is blocked',
  NeedsMainUpdate: 'the request must contain the latest view main',
}

export const autoMergeStopReasonText = (reason: RequestAutoMergeStopReason) => reasons[reason] ?? 'auto-merge stopped'
