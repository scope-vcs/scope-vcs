import assert from 'node:assert/strict'
import test from 'node:test'
import { authorRecordSummary, authorRecordTokens } from './request-author-record-model'

test('an author record leaves out zero counts and averages the rating', () => {
  assert.deepEqual(
    authorRecordTokens({
      kind: 'history',
      merged: 7,
      closed_by_maintainer: 0,
      withdrawn: 1,
      other_open: 1,
      rating: { score_sum: 9, count: 2 },
    }),
    [
      { kind: 'merged', value: '7', ratingCount: null, tip: '7 merged' },
      { kind: 'withdrawn', value: '1', ratingCount: null, tip: '1 withdrawn by the author' },
      { kind: 'open', value: '1', ratingCount: null, tip: '1 other request open' },
      { kind: 'rating', value: '4.5', ratingCount: 2, tip: 'Rated 4.5 by maintainers here, from 2 ratings' },
    ],
  )
  assert.equal(
    authorRecordSummary({
      kind: 'history',
      merged: 0,
      closed_by_maintainer: 2,
      withdrawn: 0,
      other_open: 3,
      rating: { score_sum: 2, count: 1 },
    }),
    '2 closed by a maintainer, 3 other requests open, Rated 2.0 by maintainers here, from 1 rating',
  )
  assert.equal(authorRecordSummary({ kind: 'first_request' }), 'First request in this repository')
})
