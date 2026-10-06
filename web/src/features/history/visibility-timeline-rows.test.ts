import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummaryResponse } from '@/api/types.generated'
import { visibilityTimelineBars } from './visibility-timeline-rows'

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

test('bars run oldest to newest with public above and private below the baseline', () => {
  assert.deepEqual(summarize([entry('newest', 3, 0), entry('mixed', 2, 1), entry('oldest', 0, 4)]), [
    ['oldest', 'private', -4, null],
    ['mixed', 'public', 2, null],
    ['mixed', 'private', -1, null],
    ['newest', 'public', 3, '3'],
  ])
})

test('the newest and the largest public changes are labeled', () => {
  const labels = visibilityTimelineBars([entry('newest', 1, 0), entry('largest', 282, 0), entry('other', 5, 0)])
    .flatMap((bar) => bar.label === null ? [] : [[bar.entry.id, bar.label]])
  assert.deepEqual(labels, [['largest', '282'], ['newest', '1']])
})

test('a newest change that only made paths private labels its private count', () => {
  const labels = visibilityTimelineBars([entry('newest', 0, 2), entry('older', 9, 0)])
    .flatMap((bar) => bar.label === null ? [] : [[bar.entry.id, bar.label]])
  assert.deepEqual(labels, [['older', '9'], ['newest', '2']])
})
