import assert from 'node:assert/strict'
import test from 'node:test'
import { authorRecordSummary, authorRecordTokens } from './request-author-record-model'

const rated = {
  kind: 'history',
  merged: 7,
  closed: 0,
  withdrawn: 1,
  other_open: 1,
  rating: { score_sum: 9, count: 2 },
} as const
const unrated = { kind: 'history', merged: 0, closed: 2, withdrawn: 0, other_open: 3, rating: null } as const

test('a list row leaves out zero counts and withdrawn requests and averages the rating', () => {
  assert.deepEqual(authorRecordTokens(rated), [
    { kind: 'merged', value: '7', tip: '7 merged' },
    { kind: 'open', value: '1', tip: '1 other request open' },
    { kind: 'rating', value: '4.5', tip: 'Rated 4.5 by maintainers here, from 2 ratings' },
  ])
  assert.equal(
    authorRecordTokens({ ...unrated, rating: { score_sum: 2, count: 1 } }).at(-1)?.tip,
    'Rated 2.0 by maintainers here, from 1 rating',
  )
  assert.deepEqual(authorRecordTokens({ kind: 'first_request' }).map(({ kind }) => kind), ['first'])
})

test('the request header shows one mark and keeps the counts for its card', () => {
  assert.deepEqual(authorRecordSummary(rated), {
    mark: 'rating',
    value: '4.5',
    label: 'Rated 4.5',
    lines: [
      { mark: 'merged', value: '7', text: 'merged' },
      { mark: 'withdrawn', value: '1', text: 'withdrawn' },
      { mark: 'open', value: '1', text: 'other open' },
      { mark: 'rating', value: '4.5', text: 'from 2 ratings' },
    ],
  })
  assert.deepEqual(authorRecordSummary(unrated), {
    mark: 'unrated',
    value: null,
    label: 'Not rated yet',
    lines: [
      { mark: 'closed', value: '2', text: 'closed' },
      { mark: 'open', value: '3', text: 'others open' },
      { mark: 'unrated', value: null, text: 'not rated yet' },
    ],
  })
  assert.deepEqual(authorRecordSummary({ kind: 'first_request' }), {
    mark: 'first',
    value: null,
    label: 'First request in this repository',
    lines: [{ mark: 'first', value: null, text: 'First request in this repository' }],
  })
})
