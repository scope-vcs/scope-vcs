import type { RequestAttentionReason, RequestQueueItemResponse, RequestQueueSection } from '../../api/types.generated'
import type { RequestWorkspaceItem } from './request-workspace-sidebar'

const REASONS: Record<RequestAttentionReason, string> = {
  authored: 'Your request', invited: 'Review requested', claimed: 'You’re reviewing',
  unclaimed: 'Waiting for a reviewer', new_activity: 'New reply or revision',
  restored: 'Back in your queue', snooze_expired: 'Snooze ended', waiting: 'Waiting for a reply',
  snoozed: 'Snoozed', settled: 'Settled for now', open: 'Open request',
  claimed_elsewhere: 'Being reviewed', closed: 'Closed', merged: 'Merged',
}

export function requestWorkspaceItem(item: RequestQueueItemResponse, section: RequestQueueSection, pendingId: string | null): RequestWorkspaceItem {
  const { request, attention, author } = item
  const reason = requestAttentionLabel(item)
  return {
    id: request.id, title: request.title, authorName: author.handle,
    reason, section, timeLabel: new Date(request.updated_at_unix * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }),
    actionPending: pendingId === request.id,
    canClaim: section === 'unclaimed' && attention.can_claim,
    canRestore: section === 'set_aside' && attention.can_restore,
    canSettle: section === 'active' && attention.can_set_aside,
    canSnooze: section === 'active' && attention.can_set_aside,
  }
}

export function requestAttentionLabel(item: RequestQueueItemResponse) {
  const { attention, claimer } = item
  let reason = REASONS[attention.reason]
  if (attention.reason === 'claimed_elsewhere' && claimer) reason = `Reviewing: ${claimer.handle}`
  if (attention.reason === 'snoozed' && attention.snoozed_until_unix) {
    reason = `Snoozed until ${new Date(attention.snoozed_until_unix * 1000).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`
  }
  return reason
}

export function requestSnoozeUntil(value: string, now = new Date()): number {
  const until = new Date(now)
  if (value === 'hour') until.setHours(until.getHours() + 1)
  else {
    until.setHours(9, 0, 0, 0)
    until.setDate(until.getDate() + (value === 'next_week' ? ((8 - until.getDay()) % 7 || 7) : 1))
  }
  return Math.floor(until.getTime() / 1000)
}
