import assert from 'node:assert/strict'
import test from 'node:test'
import {
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from './history-inputs'

test('normalizes an optional history cursor', () => {
  assert.deepEqual(parseHistoryPageInput({
    audience: 'private',
    before: '  cursor-50 ',
    owner: ' scope ',
    repo: ' vcs ',
  }), {
    audience: 'private',
    before: 'cursor-50',
    feed: 'updates',
    owner: 'scope',
    repo: 'vcs',
  })
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs' }).before, null)
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs' }).audience, null)
})

test('validates direct history entry and file diff requests', () => {
  assert.equal(parseHistoryEntryDetailInput({
    entry: ' update-100 ', owner: 'scope', repo: 'vcs',
  }).entry, 'update-100')
  assert.equal(parseHistoryEntryFileDiffInput({
    entry: 'update-100', owner: 'scope', path: ' /README.md ', repo: 'vcs',
  }).path, '/README.md')
  assert.throws(
    () => parseHistoryEntryDetailInput({ entry: ' ', owner: 'scope', repo: 'vcs' }),
    /history entry id is required/,
  )
})

test('defaults to pushes and merges and validates the independent feed', () => {
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs' }).feed, 'updates')
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs', feed: 'all', audience: 'public' }).feed, 'all')
  assert.throws(() => parseHistoryPageInput({ owner: 'scope', repo: 'vcs', feed: 'private' }), /Unsupported history feed/)
})

test('preserves an exact visibility effect selector without adding one to content diffs', () => {
  const request = { owner: 'scope', repo: 'vcs', entry: 'push-1', path: '/same.ts' }
  assert.equal(parseHistoryEntryFileDiffInput(request).visibility_change, null)
  assert.equal(parseHistoryEntryFileDiffInput({ ...request, visibility_change: ' change-2 ' }).visibility_change, 'change-2')
  assert.throws(() => parseHistoryEntryFileDiffInput({ ...request, visibility_change: 12 }), /visibility change id/)
})
