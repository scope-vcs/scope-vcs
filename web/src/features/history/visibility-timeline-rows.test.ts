import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummaryResponse } from '@/api/types.generated'
import { VISIBILITY_TIMELINE_ENTRY_LIMIT, visibilityTimelineBars } from './visibility-timeline-rows'

const entry = (id: string, madePublic: number, madePrivate: number): HistoryEntrySummaryResponse => ({
  author: 'maya',
  file_change_count: madePublic + madePrivate,
  id,
  kind: 'visibility_change',
  message: `Change ${id}`,
  occurred_at_unix: null,
  parent_id: null,
  source_id: `source-${id}`,
  visibility_summary: { made_private_count: madePrivate, made_public_count: madePublic },
})

const summarize = (newestFirst: HistoryEntrySummaryResponse[]) =>
  visibilityTimelineBars(newestFirst).map((bar) => [bar.entry.id, bar.direction, bar.signedCount, bar.label])

const labels = (newestFirst: HistoryEntrySummaryResponse[]) =>
  visibilityTimelineBars(newestFirst).flatMap((bar) => bar.label === null ? [] : [[bar.entry.id, bar.label]])

test('bars run oldest to newest with public above and private below the baseline', () => {
  assert.deepEqual(summarize([entry('newest', 3, 0), entry('mixed', 2, 1), entry('oldest', 0, 4)]), [
    ['oldest', 'private', -4, '4'],
    ['mixed', 'public', 2, null],
    ['mixed', 'private', -1, null],
    ['newest', 'public', 3, '3'],
  ])
})

test('the newest and the largest changes are labeled', () => {
  assert.deepEqual(labels([entry('newest', 1, 0), entry('largest', 282, 0), entry('other', 5, 0)]), [
    ['largest', '282'],
    ['newest', '1'],
  ])
})

test('a history that only made paths private labels its largest private change', () => {
  assert.deepEqual(labels([entry('newest', 0, 2), entry('largest', 0, 282), entry('oldest', 0, 1)]), [
    ['largest', '282'],
    ['newest', '2'],
  ])
})

test('older pages loaded into the shared feed stay out of the timeline', () => {
  const newestFirst = Array.from({ length: VISIBILITY_TIMELINE_ENTRY_LIMIT + 10 }, (_, index) => entry(`e${index}`, 1, 0))
  const ids = visibilityTimelineBars(newestFirst).map((bar) => bar.entry.id)
  assert.equal(ids.length, VISIBILITY_TIMELINE_ENTRY_LIMIT)
  assert.equal(ids.at(0), `e${VISIBILITY_TIMELINE_ENTRY_LIMIT - 1}`)
  assert.equal(ids.at(-1), 'e0')
})
