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
  RequestQueueNextStep,
} from '../../api/types.generated'
import { requestMergeabilityLabel } from './request-labels'

const REASONS: Record<Exclude<RequestAttentionReason, 'authored'>, string> = {
  invited: 'Review requested',
  claimed: 'You’re reviewing',
  unclaimed: 'Waiting for a reviewer',
  new_activity: 'New reply or revision',
  restored: 'Back in your queue',
  snooze_expired: 'Woke',
  waiting: 'Waiting for a reply',
  snoozed: 'Snoozed',
  settled: 'Settled for now',
  open: 'Open request',
  claimed_elsewhere: 'Being reviewed',
  closed: 'Closed',
  merged: 'Merged',
}

export function requestStatusLabel(item: RequestQueueItemResponse, hydrated: boolean) {
  const { attention, claimer, request } = item
  if (attention.reason === 'authored') {
    const mergeability = requestMergeabilityLabel(request)
    return request.mergeability.status === 'NotMaintainer' ? (REQUEST_NEXT_STEP_ACTIONS[item.next_step] ?? mergeability) : mergeability
  }
  if (attention.reason === 'claimed_elsewhere' && claimer) return `Reviewing: ${claimer.handle}`
  if (attention.reason === 'snoozed' && attention.snoozed_until_unix) {
    return `Snoozed until ${formatUnixSnoozeUntil(attention.snoozed_until_unix, hydrated)}`
  }
  return REASONS[attention.reason]
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

export const REQUEST_NEXT_STEP_ACTIONS: Partial<Record<RequestQueueNextStep, string>> = {
  merge: 'Ready to merge',
  fix_checks: 'Checks need attention',
  update: 'Needs an update',
}

export function requestStackParentLabel({ request }: RequestQueueItemResponse) {
  if (!request.stack_parent) return undefined
  return request.stack_parent.kind === 'available' ? `Based on ${request.stack_parent.request.title}` : 'Unavailable dependency'
}

export function requestHasNewActivity(item: RequestQueueItemResponse) {
  return item.attention.reason === 'new_activity'
}

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const WEEK = 7 * DAY

export function requestAttentionStale(attentionAtUnix: number, nowUnix: number) {
  return nowUnix - attentionAtUnix >= DAY
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

type RequestSnoozeOption = (typeof REQUEST_SNOOZE_OPTIONS)[number]['value']

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

export type CustomSnoozeInput =
  | { mode: 'date'; date: string; time: string }
  | { mode: 'duration'; amount: string; unit: 'minutes' | 'hours' | 'days' }

export function resolveCustomSnooze(input: CustomSnoozeInput, now: Date): number | null {
  let wake: Date
  if (input.mode === 'duration') {
    const amount = Number(input.amount)
    if (!Number.isFinite(amount) || amount <= 0) return null
    wake = new Date(now.getTime() + amount * { minutes: MINUTE, hours: HOUR, days: DAY }[input.unit] * 1_000)
  } else {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(input.date) || !/^\d{2}:\d{2}$/.test(input.time)) return null
    wake = new Date(`${input.date}T${input.time}:00`)
    if (localSnoozeDate(wake) !== input.date || localSnoozeTime(wake) !== input.time) return null
  }
  const until = Math.floor(wake.getTime() / 1_000)
  return Number.isFinite(until) && until > now.getTime() / 1_000 ? until : null
}

export function localSnoozeDate(date: Date): string {
  return `${String(date.getFullYear()).padStart(4, '0')}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

export function localSnoozeTime(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}
