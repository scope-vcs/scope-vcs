import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEventResponse } from '../../api/types.generated'
import { requestRevisionPushes, searchRequestRevisionPushes } from './request-revision-pushes'

const actor = { handle: 'maya', id: 'user_maya' } as RequestEventResponse['actor']

function pushEvent(position: number, note: string | null = null): RequestEventResponse {
  return {
    actor,
    created_at_unix: 1_700_000_000 + position,
    id: `event_${position}`,
    kind: 'RevisionPushed',
    payload: { RevisionPushed: { new_head_oid: `a1b2c3${position}`, note, old_head_oid: `f0e0d0${position}` } },
    position,
  }
}

const pushes = requestRevisionPushes([
  pushEvent(2, 'Name the retry cap'),
  { ...pushEvent(3), kind: 'Submitted', payload: { Submitted: { head_oid: 'abc' } } },
  pushEvent(8, 'Document the retry policy'),
  pushEvent(5, 'Add bounded jitter'),
])

test('only revision pushes are listed, newest first, keyed by their revision id', () => {
  assert.deepEqual(pushes.map(({ id, position }) => [id, position]), [
    ['event_8', 8],
    ['event_5', 5],
    ['event_2', 2],
  ])
})

test('search matches every word against number, pusher, note and heads', () => {
  const positions = (query: string) => searchRequestRevisionPushes(pushes, query).map(({ position }) => position)
  assert.deepEqual(positions(''), [8, 5, 2])
  assert.deepEqual(positions('retry'), [8, 2])
  assert.deepEqual(positions('Retry  POLICY'), [8])
  assert.deepEqual(positions('revision 5'), [5])
  assert.deepEqual(positions('maya jitter'), [5])
  assert.deepEqual(positions('a1b2c32'), [2])
  assert.deepEqual(positions('nothing here'), [])
})
