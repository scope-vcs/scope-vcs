import type { RequestAttentionReason, RequestQueueItemResponse } from '../../api/types.generated'

const REASONS: Record<RequestAttentionReason, string> = {
  authored: 'Your request',
  invited: 'Review requested',
  claimed: 'You’re reviewing',
  unclaimed: 'Waiting for a reviewer',
  new_activity: 'New reply or revision',
  restored: 'Back in your queue',
  snooze_expired: 'Snooze ended',
  waiting: 'Waiting for a reply',
  snoozed: 'Snoozed',
  settled: 'Settled for now',
  open: 'Open request',
  claimed_elsewhere: 'Being reviewed',
  closed: 'Closed',
  merged: 'Merged',
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

export const REQUEST_SNOOZE_OPTIONS = [
  { label: 'In an hour', value: 'hour', detail: null },
  { label: 'Tomorrow', value: 'tomorrow', detail: '9:00 AM' },
  { label: 'Next week', value: 'next_week', detail: 'Monday, 9:00 AM' },
] as const

export function requestSnoozeUntil(
  value: (typeof REQUEST_SNOOZE_OPTIONS)[number]['value'],
  now = new Date(),
): number {
  const until = new Date(now)
  if (value === 'hour') until.setHours(until.getHours() + 1)
  else {
    until.setHours(9, 0, 0, 0)
    until.setDate(until.getDate() + (value === 'next_week' ? (8 - until.getDay()) % 7 || 7 : 1))
  }
  return Math.floor(until.getTime() / 1000)
}
