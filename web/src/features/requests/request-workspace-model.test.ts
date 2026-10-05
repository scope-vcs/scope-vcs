import assert from 'node:assert/strict'
import test from 'node:test'
import {
  requestAgeLabel,
  requestAttentionHeat,
  requestSnoozeLandingLabel,
  requestSnoozeUntil,
} from './request-workspace-model'

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

