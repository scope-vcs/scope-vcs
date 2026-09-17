import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse } from '../../api/types.generated'
import { applyAttentionMoves, isInstantCommand, movedRow, queueReflectsMove } from './request-attention-moves'
import type { RequestQueuePages } from './request-list-model'

function row(id: string, overrides: Partial<RequestQueueItemResponse['attention']> = {}) {
  return {
    attention_at_unix: 100,
    request: { id },
    author: { handle: 'dev' },
    claimer: null,
    attention: {
      state: 'active', reason: 'authored', activity_version: 3, through_activity_version: 3, revision: 4,
      snoozed_until_unix: null, can_claim: false, can_set_aside: true, can_restore: false, can_release: false,
      ...overrides,
    },
  } as unknown as RequestQueueItemResponse
}

function pages(sections: Partial<Record<keyof RequestQueuePages, RequestQueueItemResponse[]>>): RequestQueuePages {
  const page = (requests: RequestQueueItemResponse[] = []) => ({ requests, next_cursor: null, next_attention_at_unix: null })
  return { active: page(sections.active), unclaimed: page(sections.unclaimed), set_aside: page(sections.set_aside), done: page(sections.done) }
}

const ids = (list: RequestQueueItemResponse[]) => list.map((item) => item.request.id)

test('only settle, snooze and restore apply in the browser', () => {
  assert.equal(isInstantCommand({ action: 'settle' }), true)
  assert.equal(isInstantCommand({ action: 'snooze' }), true)
  assert.equal(isInstantCommand({ action: 'restore' }), true)
  assert.equal(isInstantCommand({ action: 'claim' }), false)
  assert.equal(isInstantCommand({ action: 'release' }), false)
})

test('a settled row leaves active at once and heads set aside', () => {
  const a = row('a')
  const loaded = pages({ active: [a, row('b')], set_aside: [row('c', { reason: 'settled' })] })
  const shown = applyAttentionMoves(loaded, [{ item: a, command: { action: 'settle' }, atUnix: 500, confirmed: null }])
  assert.deepEqual(ids(shown.active.requests), ['b'])
  assert.deepEqual(ids(shown.set_aside.requests), ['a', 'c'])
  const moved = shown.set_aside.requests[0].attention
  assert.equal(moved.reason, 'settled')
  assert.equal(moved.can_restore, true)
  assert.equal(moved.can_set_aside, false)
  assert.deepEqual(ids(loaded.active.requests), ['a', 'b'], 'loaded pages stay untouched')
})

test('a snoozed row carries its wake time', () => {
  const moved = movedRow({ item: row('a'), command: { action: 'snooze', until_unix: 900 }, atUnix: 500, confirmed: null })
  assert.equal(moved.section, 'set_aside')
  assert.equal(moved.item.attention.reason, 'snoozed')
  assert.equal(moved.item.attention.snoozed_until_unix, 900)
})

test('a restored row returns to active and can be set aside again', () => {
  const a = row('a', { state: 'settled', reason: 'settled', can_set_aside: false, can_restore: true })
  const shown = applyAttentionMoves(pages({ set_aside: [a] }), [{ item: a, command: { action: 'restore' }, atUnix: 500, confirmed: null }])
  assert.deepEqual(ids(shown.set_aside.requests), [])
  assert.deepEqual(ids(shown.active.requests), ['a'])
  assert.equal(shown.active.requests[0].attention.can_set_aside, true)
})

test('a move holds even when a stale refresh still lists the row in its old section', () => {
  const a = row('a')
  const move = { item: a, command: { action: 'settle' }, atUnix: 500, confirmed: null } as const
  const shown = applyAttentionMoves(pages({ active: [a], set_aside: [a] }), [move])
  assert.deepEqual(ids(shown.active.requests), [])
  assert.deepEqual(ids(shown.set_aside.requests), ['a'])
})

test('a moved row lands where the API will serve it, not always on top', () => {
  const later = { ...row('z'), attention_at_unix: 900 }
  const tied = { ...row('b'), attention_at_unix: 500 }
  const a = row('a', { state: 'settled', reason: 'settled', can_restore: true })
  const shown = applyAttentionMoves(pages({ active: [later, tied], set_aside: [a] }), [
    { item: a, command: { action: 'restore' }, atUnix: 500, confirmed: null },
  ])
  assert.deepEqual(ids(shown.active.requests), ['z', 'a', 'b'])
})

test('a move never pulls a row behind its own last update', () => {
  const future = { ...row('a'), attention_at_unix: 2_000 }
  const moved = movedRow({ item: future, command: { action: 'settle' }, atUnix: 500, confirmed: null })
  assert.equal(moved.item.attention_at_unix, 2_000)
})

test('a move holds until the loaded row reaches the answered revision', () => {
  const a = row('a')
  const answer = { attention: { ...a.attention, state: 'settled', reason: 'settled', revision: 5 }, claimer: null } as never
  const unanswered = { item: a, command: { action: 'settle' }, atUnix: 500, confirmed: null } as const
  const answered = { ...unanswered, confirmed: answer }
  const stale = pages({ active: [a] })
  const caughtUp = pages({ set_aside: [row('a', { state: 'settled', reason: 'settled', revision: 5 })] })

  assert.equal(queueReflectsMove(caughtUp, unanswered), false, 'an unanswered move always holds')
  assert.equal(queueReflectsMove(stale, answered), false, 'a reload from before the answer cannot undo it')
  assert.deepEqual(ids(applyAttentionMoves(stale, [answered]).active.requests), [])
  assert.equal(queueReflectsMove(caughtUp, answered), true)
  assert.equal(applyAttentionMoves(caughtUp, [answered]), caughtUp, 'a reflected move changes nothing')
  assert.equal(queueReflectsMove(pages({}), answered), true, 'a row the queue no longer loads has no stale copy')
})

test('the server answer replaces the prediction', () => {
  const a = row('a')
  const confirmed = { attention: { ...a.attention, state: 'settled', reason: 'settled', activity_version: 4 }, claimer: null } as never
  const moved = movedRow({ item: a, command: { action: 'settle' }, atUnix: 500, confirmed })
  assert.equal(moved.item.attention.activity_version, 4)
})
