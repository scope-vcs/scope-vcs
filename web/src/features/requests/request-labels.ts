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
  Open: { label: 'Open', tone: 'info' },
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
  NotMaintainer: { label: 'Maintainer required', tone: 'neutral' },
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
  const payload = event.payload as unknown as Record<
    string,
    Record<string, unknown>
  >
  const value = payload[event.kind]
  if (!value) return null
  switch (event.kind) {
    case 'Started':
      return 'Initial request identity recorded.'
    case 'Submitted':
      return oidText(value.head_oid)
    case 'RevisionPushed':
      return [
        `${oidText(value.old_head_oid)} → ${oidText(value.new_head_oid)}`,
        stringValue(value.note),
      ]
        .filter(Boolean)
        .join('\n')
    case 'Closed':
      return oidText(value.head_oid)
    case 'Merged':
      return `${oidText(value.head_oid)} → ${oidText(value.main_oid)}`
    case 'IdentityEdited':
      return 'The request title or description was updated.'
    case 'DiscussionResolved':
    case 'DiscussionReopened':
      return value.discussion_id
        ? `Discussion ${stringValue(value.discussion_id)}`
        : null
  }
}

function oidText(value: unknown) {
  return typeof value === 'string' ? shortOid(value) : null
}

function stringValue(value: unknown) {
  return typeof value === 'string' && value.trim() ? value : null
}
