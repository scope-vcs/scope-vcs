import {
  formatUnixClockTime,
  formatUnixMonthDay,
  formatUnixSnoozeUntil,
  formatUnixWeekdayTime,
} from '../../lib/date-format'
import type {
  RequestAttentionReason,
  RequestQueueGroup,
  RequestQueueItemResponse,
} from '../../api/types.generated'

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

export function requestAttentionLabel(item: RequestQueueItemResponse, hydrated: boolean) {
  const { attention, claimer } = item
  let reason = REASONS[attention.reason]
  if (attention.reason === 'claimed_elsewhere' && claimer) reason = `Reviewing: ${claimer.handle}`
  if (attention.reason === 'snoozed' && attention.snoozed_until_unix) {
    reason = `Snoozed until ${formatUnixSnoozeUntil(attention.snoozed_until_unix, hydrated)}`
  }
  return reason
}

export type RequestAttentionGroup = RequestQueueGroup

export const REQUEST_ATTENTION_GROUP_ORDER = [
  'needs_you',
  'waiting',
  'unclaimed',
  'set_aside',
  'done',
] as const satisfies readonly RequestAttentionGroup[]

export const REQUEST_ATTENTION_GROUP_LABELS: Record<RequestAttentionGroup, string> = {
  needs_you: 'Needs you',
  waiting: 'Waiting on others',
  unclaimed: 'Unclaimed',
  set_aside: 'Set aside',
  done: 'Done',
}

export function requestHasNewActivity(item: RequestQueueItemResponse) {
  return item.attention.reason === 'new_activity'
}

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const WEEK = 7 * DAY

export function requestAttentionHeat(attentionAtUnix: number, nowUnix: number): 0 | 1 | 2 | 3 {
  const age = nowUnix - attentionAtUnix
  if (age < DAY) return 0
  if (age < 3 * DAY) return 1
  if (age < WEEK) return 2
  return 3
}

export function requestAgeLabel(attentionAtUnix: number, nowUnix: number, hydrated: boolean) {
  const age = Math.max(0, nowUnix - attentionAtUnix)
  if (age < MINUTE) return 'now'
  if (age < HOUR) return `${Math.floor(age / MINUTE)}m`
  if (age < DAY) return `${Math.floor(age / HOUR)}h`
  if (age < 4 * WEEK) {
    const days = Math.floor(age / DAY)
    return days < 7 ? `${days}d` : `${Math.floor(days / 7)}w`
  }
  return formatUnixMonthDay(attentionAtUnix, hydrated)
}

export const REQUEST_SNOOZE_OPTIONS = [
  { label: 'In an hour', value: 'hour' },
  { label: 'Tomorrow', value: 'tomorrow' },
  { label: 'Next week', value: 'next_week' },
] as const

export type RequestSnoozeOption = (typeof REQUEST_SNOOZE_OPTIONS)[number]['value']

export function requestSnoozeUntil(value: RequestSnoozeOption, now = new Date()): number {
  const until = new Date(now)
  if (value === 'hour') until.setHours(until.getHours() + 1)
  else {
    until.setHours(9, 0, 0, 0)
    until.setDate(until.getDate() + (value === 'next_week' ? (8 - until.getDay()) % 7 || 7 : 1))
  }
  return Math.floor(until.getTime() / 1000)
}

export function requestSnoozeLandingLabel(value: RequestSnoozeOption, now: Date): string {
  const until = requestSnoozeUntil(value, now)
  return value === 'hour' ? formatUnixClockTime(until, true) : formatUnixWeekdayTime(until, true)
}
