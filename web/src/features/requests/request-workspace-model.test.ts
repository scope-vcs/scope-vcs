import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestAttentionReason } from '../../api/types.generated'
import { requestAgeLabel, requestAttentionGroup } from './request-workspace-model'

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
    assert.equal(requestAttentionGroup('active', reason as RequestAttentionReason), group, reason)
  }
})

test('storage sections outside active map straight to their group', () => {
  assert.equal(requestAttentionGroup('unclaimed', 'unclaimed'), 'unclaimed')
  assert.equal(requestAttentionGroup('set_aside', 'snoozed'), 'set_aside')
  assert.equal(requestAttentionGroup('set_aside', 'settled'), 'set_aside')
  assert.equal(requestAttentionGroup('done', 'merged'), 'done')
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
