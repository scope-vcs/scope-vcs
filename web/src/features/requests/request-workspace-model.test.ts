import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestAttentionReason } from '../../api/types.generated'
import {
  requestAgeLabel,
  requestAttentionGroup,
  requestAttentionHeat,
  requestSnoozeLandingLabel,
  requestSnoozeUntil,
} from './request-workspace-model'

const ACTIVE_REASONS: Record<RequestAttentionReason, 'needs_you' | 'waiting'> = {
  invited: 'needs_you',
  claimed: 'needs_you',
  new_activity: 'needs_you',
  restored: 'needs_you',
  snooze_expired: 'needs_you',
  authored: 'waiting',
  waiting: 'waiting',
  open: 'waiting',
  claimed_elsewhere: 'waiting',
  unclaimed: 'waiting',
  snoozed: 'waiting',
  settled: 'waiting',
  closed: 'waiting',
  merged: 'waiting',
}

test('every active reason lands in needs-you or waiting', () => {
  for (const [reason, group] of Object.entries(ACTIVE_REASONS)) {
    assert.equal(requestAttentionGroup('active', reason as RequestAttentionReason, false), group, reason)
  }
})

test('a maintainer’s own request needs them, a contributor’s waits', () => {
  assert.equal(requestAttentionGroup('active', 'authored', true), 'needs_you')
  assert.equal(requestAttentionGroup('active', 'authored', false), 'waiting')
  assert.equal(requestAttentionGroup('active', 'waiting', true), 'waiting')
  assert.equal(requestAttentionGroup('active', 'invited', true), 'needs_you')
})

test('storage sections outside active map straight to their group', () => {
  assert.equal(requestAttentionGroup('unclaimed', 'unclaimed', true), 'unclaimed')
  assert.equal(requestAttentionGroup('set_aside', 'snoozed', true), 'set_aside')
  assert.equal(requestAttentionGroup('set_aside', 'settled', true), 'set_aside')
  assert.equal(requestAttentionGroup('done', 'merged', true), 'done')
})

test('row age reads as a compact unit', () => {
  const now = 1_800_000_000
  assert.equal(requestAgeLabel(now - 20, now, true), 'now')
  assert.equal(requestAgeLabel(now - 5 * 60, now, true), '5m')
  assert.equal(requestAgeLabel(now - 4 * 3600, now, true), '4h')
  assert.equal(requestAgeLabel(now - 2 * 86400, now, true), '2d')
  assert.equal(requestAgeLabel(now - 13 * 86400, now, true), '1w')
  assert.equal(requestAgeLabel(now + 500, now, true), 'now')
  assert.match(requestAgeLabel(now - 90 * 86400, now, true), /^[A-Z][a-z]{2} \d{1,2}$/)
  assert.match(requestAgeLabel(now - 90 * 86400, now, false), /^[A-Z][a-z]{2} \d{1,2}$/)
})

test('attention heat steps up with the wait', () => {
  const now = 1_800_000_000
  assert.equal(requestAttentionHeat(now + 60, now), 0)
  assert.equal(requestAttentionHeat(now - 3600, now), 0)
  assert.equal(requestAttentionHeat(now - 2 * 86400, now), 1)
  assert.equal(requestAttentionHeat(now - 5 * 86400, now), 2)
  assert.equal(requestAttentionHeat(now - 30 * 86400, now), 3)
})

test('snooze choices land an hour on, tomorrow at nine, or next Monday at nine', () => {
  const wednesday = new Date('2026-09-23T14:12:00Z')
  const hour = new Date(requestSnoozeUntil('hour', wednesday) * 1_000)
  const tomorrow = new Date(requestSnoozeUntil('tomorrow', wednesday) * 1_000)
  const nextWeek = new Date(requestSnoozeUntil('next_week', wednesday) * 1_000)
  assert.equal(hour.toISOString(), '2026-09-23T15:12:00.000Z')
  assert.equal(tomorrow.toISOString(), '2026-09-24T09:00:00.000Z')
  assert.equal(nextWeek.toISOString(), '2026-09-28T09:00:00.000Z')
})

test('the menu words each landing as a clock today and a weekday later', () => {
  const wednesday = new Date('2026-09-23T14:12:00Z')
  assert.equal(requestSnoozeLandingLabel('hour', wednesday), '3:12 PM')
  assert.equal(requestSnoozeLandingLabel('tomorrow', wednesday), 'Thu 9:00 AM')
  assert.equal(requestSnoozeLandingLabel('next_week', wednesday), 'Mon 9:00 AM')
})

