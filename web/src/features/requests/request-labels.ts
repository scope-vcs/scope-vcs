import type { BadgeVariant } from '@/components/ui/badge'
import type { RepoViews } from '../../api/repo-views'
import { shortOid } from '../../lib/short-oid'
import type {
  RequestCheckEvaluationState,
  RequestChecksResponse,
  RequestEventResponse,
  RequestListItemResponse,
  RequestSummaryResponse,
  RequestEventKind,
} from '@/api/types.generated'
import { autoMergeStopReasonText } from './request-auto-merge-model'

const EVENT_LABELS = {
  Started: 'Started',
  Submitted: 'Submitted',
  RevisionPushed: 'Revision pushed',
  Merged: 'Merged',
  Closed: 'Closed',
  IdentityEdited: 'Request edited',
  DiscussionResolved: 'Discussion resolved',
  DiscussionReopened: 'Discussion reopened',
  AutoMergeEnabled: 'Auto-merge enabled',
  AutoMergeCancelled: 'Auto-merge canceled',
  AutoMergeStopped: 'Auto-merge stopped',
  AutoMergeFulfilled: 'Merged automatically',
} as const satisfies Record<RequestEventKind, string>

const MERGEABILITY = {
  Ready: { label: 'Ready to merge', tone: 'success' },
  Draft: { label: 'Draft', tone: 'neutral' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
  NotMaintainer: { label: 'Maintainer merges', tone: 'outline' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
  ChecksNotEvaluated: { label: 'CI status unavailable', tone: 'warning' },
  ChecksAwaitingApproval: { label: 'CI needs permission to run', tone: 'warning' },
  ChecksPending: { label: 'Waiting for CI', tone: 'info' },
  ChecksFailed: { label: 'CI failed', tone: 'danger' },
  ChecksConfigurationError: { label: 'CI needs configuration', tone: 'danger' },
} as const satisfies Record<
  RequestSummaryResponse['mergeability']['status'],
  { label: string; tone: BadgeVariant }
>

const CHECK_EVALUATION_NOTES = {
  'no-checks': null,
  'awaiting-approval': 'A maintainer must allow CI to run for this revision.',
  'started': null,
  'configuration-error': null,
} as const satisfies Record<RequestCheckEvaluationState, string | null>

type RequestLabelSource = RequestSummaryResponse | RequestListItemResponse

export function requestAuthorRoleLabel(request: RequestLabelSource) {
  switch (request.author_role) {
    case 'Owner':
      return 'Owner'
    case 'Member':
      return 'Member'
    case 'Public':
      return 'Public contributor'
  }
}

export function requestViewLabel(request: RequestLabelSource, views: Pick<RepoViews, 'name'>) {
  return `${views.name(request.view)} request`
}

export function eventKindLabel(kind: RequestEventKind) {
  return EVENT_LABELS[kind]
}

export function requestMergeabilityLabel(request: RequestLabelSource) {
  return MERGEABILITY[request.mergeability.status].label
}

export function requestMergeabilityTone(request: RequestLabelSource): BadgeVariant {
  return MERGEABILITY[request.mergeability.status].tone
}

export function requestCheckEvaluationNote(checks: RequestChecksResponse) {
  if (checks.state === null) {
    return 'CI status is not available yet.'
  }
  if (checks.message) return checks.message
  if (checks.state === 'configuration-error') {
    return 'The CI configuration for this revision is invalid.'
  }
  return CHECK_EVALUATION_NOTES[checks.state]
}

export function requestChecksWorkflowWarning(checks: RequestChecksResponse) {
  return checks.can_approve && checks.changes_github_workflows
    ? 'This revision changes GitHub workflow files. Allowing CI runs them with your repository’s secrets.'
    : null
}

export function requestPublicChecksNote(checks: RequestChecksResponse, requestViewName: string) {
  return checks.private_request_on_public_github
    ? `CI runs publicly, so this ${requestViewName} request’s changes are public.`
    : null
}

export function requestEventBody(event: RequestEventResponse) {
  const payload = event.payload
  if ('Started' in payload) return 'Initial request identity recorded.'
  if ('Submitted' in payload) return shortOid(payload.Submitted.head_oid)
  if ('RevisionPushed' in payload) {
    const { old_head_oid, new_head_oid, note } = payload.RevisionPushed
    return [
      `${shortOid(old_head_oid)} → ${shortOid(new_head_oid)}`,
      note?.trim() ? note : null,
    ]
      .filter(Boolean)
      .join('\n')
  }
  if ('Closed' in payload) return shortOid(payload.Closed.head_oid)
  if ('Merged' in payload) {
    return `${shortOid(payload.Merged.head_oid)} → ${shortOid(payload.Merged.main_oid)}`
  }
  if ('IdentityEdited' in payload) {
    return 'The request title or description was updated.'
  }
  if ('DiscussionResolved' in payload) {
    return discussionText(payload.DiscussionResolved.discussion_id)
  }
  if ('DiscussionReopened' in payload) {
    return discussionText(payload.DiscussionReopened.discussion_id)
  }
  if ('AutoMergeEnabled' in payload) {
    return `Will merge ${shortOid(payload.AutoMergeEnabled.head_oid)} when this revision is ready to merge.`
  }
  if ('AutoMergeCancelled' in payload) {
    return `Canceled auto-merge for ${shortOid(payload.AutoMergeCancelled.head_oid)}.`
  }
  if ('AutoMergeStopped' in payload) {
    const { head_oid, reason } = payload.AutoMergeStopped
    return `Auto-merge stopped for ${shortOid(head_oid)}: ${autoMergeStopReasonText(reason)}.`
  }
  if ('AutoMergeFulfilled' in payload) {
    const { head_oid, main_oid } = payload.AutoMergeFulfilled
    return `${shortOid(head_oid)} → ${shortOid(main_oid)}`
  }
  payload satisfies never
  return null
}

function discussionText(discussionId: string) {
  return discussionId ? `Discussion ${discussionId}` : null
}
