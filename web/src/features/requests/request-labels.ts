import type { BadgeVariant } from '@/components/ui/badge'
import { shortOid } from '../../lib/short-oid'
import type {
  RequestCheckEvaluationState,
  RequestChecksResponse,
  RequestEventResponse,
  RequestListItemResponse,
  RequestSummaryResponse,
  RequestEventKind,
  RequestState,
} from '@/api/types.generated'
import { autoMergeStopReasonText } from './request-auto-merge-model'

const REQUEST_STATES = {
  Draft: { label: 'Draft', tone: 'neutral' },
  Open: { label: 'Open', tone: 'success' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
} as const satisfies Record<
  RequestState,
  { label: string; tone: BadgeVariant }
>

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
  Ready: { label: 'Clean merge available', tone: 'success' },
  Draft: { label: 'Draft', tone: 'neutral' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
  NotMaintainer: { label: 'Maintainer merges', tone: 'outline' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
  ChecksNotEvaluated: { label: 'Checks not worked out', tone: 'warning' },
  ChecksAwaitingApproval: { label: 'Checks await approval', tone: 'warning' },
  ChecksPending: { label: 'Checks running', tone: 'info' },
  ChecksFailed: { label: 'Checks failed', tone: 'danger' },
  ChecksConfigurationError: { label: 'Checks misconfigured', tone: 'danger' },
} as const satisfies Record<
  RequestSummaryResponse['mergeability']['status'],
  { label: string; tone: BadgeVariant }
>

// What the evaluation itself says, when it is not simply the runs and their states.
const CHECK_EVALUATION_NOTES = {
  'no-checks': 'This head asks for no checks.',
  'awaiting-approval': 'These checks wait for a maintainer to start them.',
  'started': null,
  'configuration-error': null,
} as const satisfies Record<RequestCheckEvaluationState, string | null>

type RequestLabelSource = RequestSummaryResponse | RequestListItemResponse

export function requestStatusLabel(request: RequestLabelSource) {
  return REQUEST_STATES[request.state].label
}

export function requestStatusTone(request: RequestLabelSource): BadgeVariant {
  return REQUEST_STATES[request.state].tone
}

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

export function requestAudienceLabel(request: RequestLabelSource) {
  return request.audience === 'Private' ? 'Private request' : 'Public request'
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
  if (checks.state === 'configuration-error') {
    return checks.message ?? 'This head’s workflow configuration is invalid.'
  }
  if (checks.state === null) {
    return 'The checks for this commit have not been worked out yet.'
  }
  return CHECK_EVALUATION_NOTES[checks.state]
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
    return `Will merge ${shortOid(payload.AutoMergeEnabled.head_oid)} when checks pass.`
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
  // Exhaustive: a new payload variant from Rust lands here as a type error.
  payload satisfies never
  return null
}

function discussionText(discussionId: string) {
  return discussionId ? `Discussion ${discussionId}` : null
}
