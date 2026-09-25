import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestEventResponse } from '../../api/types.generated'
import { requestRevisionPushes, requestTimelineItems } from './request-timeline-items'

const actor = { handle: 'maya', id: 'user_maya' } as RequestEventResponse['actor']

function pushEvent(position: number): RequestEventResponse {
  return {
    actor,
    created_at_unix: 1_700_000_000 + position,
    id: `event_${position}`,
    kind: 'RevisionPushed',
    payload: { RevisionPushed: { new_head_oid: `new${position}`, note: null, old_head_oid: `old${position}` } },
    position,
  }
}

const pushes = requestRevisionPushes([
  pushEvent(8),
  { ...pushEvent(3), kind: 'Submitted', payload: { Submitted: { head_oid: 'abc' } } },
  pushEvent(2),
  pushEvent(5),
])

function order(items: ReturnType<typeof requestTimelineItems<{ opened_position: number }>>) {
  return items.map((item) => item.kind === 'revision'
    ? `push ${item.push.position}`
    : `discussion ${item.discussion.opened_position}`)
}

test('only revision pushes become timeline pushes, keyed by their revision id', () => {
  assert.deepEqual(pushes.map(({ id, position }) => [id, position]), [
    ['event_8', 8],
    ['event_2', 2],
    ['event_5', 5],
  ])
})

test('pushes fall between discussions by activity position', () => {
  const discussions = [{ opened_position: 4 }, { opened_position: 6 }]
  assert.deepEqual(order(requestTimelineItems(discussions, pushes, false)), [
    'push 2',
    'discussion 4',
    'push 5',
    'discussion 6',
    'push 8',
  ])
})

test('pushes older than the loaded discussions wait for Load earlier', () => {
  const discussions = [{ opened_position: 4 }, { opened_position: 6 }]
  assert.deepEqual(order(requestTimelineItems(discussions, pushes, true)), [
    'discussion 4',
    'push 5',
    'discussion 6',
    'push 8',
  ])
  assert.deepEqual(order(requestTimelineItems([], pushes, true)), [])
})

test('a request without discussions still lists its pushes, oldest first', () => {
  assert.deepEqual(order(requestTimelineItems([], pushes, false)), ['push 2', 'push 5', 'push 8'])
})

test('a discussion still being posted stays after every push', () => {
  const discussions = [{ opened_position: Number.MAX_SAFE_INTEGER }]
  assert.deepEqual(order(requestTimelineItems(discussions, pushes, false)), [
    'push 2',
    'push 5',
    'push 8',
    `discussion ${Number.MAX_SAFE_INTEGER}`,
  ])
})
