import type { BadgeVariant } from '@/components/ui/badge'
import { shortOid } from '../../lib/short-oid'
import type {
  RequestEventResponse,
  RequestListItemResponse,
  RequestSummaryResponse,
  RequestEventKind,
  RequestState,
} from '@/api/types.generated'

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
} as const satisfies Record<RequestEventKind, string>

const MERGEABILITY = {
  Ready: { label: 'Clean merge available', tone: 'success' },
  Draft: { label: 'Draft', tone: 'neutral' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
  NotMaintainer: { label: 'Maintainer merges', tone: 'outline' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
} as const satisfies Record<
  RequestSummaryResponse['mergeability']['status'],
  { label: string; tone: BadgeVariant }
>

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
  // Exhaustive: a new payload variant from Rust lands here as a type error.
  payload satisfies never
  return null
}

function discussionText(discussionId: string) {
  return discussionId ? `Discussion ${discussionId}` : null
}
