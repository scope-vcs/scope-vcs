import assert from 'node:assert/strict'
import test from 'node:test'
import type { ReviewFileDiff } from '../../api/types'
import {
  reviewFileDiffEmptyLabel,
} from './review-file-diff-presentation'

function emptyDiff(overrides: Partial<ReviewFileDiff> = {}): ReviewFileDiff {
  return {
    kind: 'Modified',
    new_mode: '100644',
    old_mode: '100644',
    path: '/fixture.txt',
    presentation: { kind: 'empty' },
    ...overrides,
  }
}

test('preserves zero-hunk and mode-only labels', () => {
  assert.equal(reviewFileDiffEmptyLabel(emptyDiff()), 'No content changes')
  assert.equal(
    reviewFileDiffEmptyLabel(emptyDiff({ new_mode: '100755' })),
    'Mode 100644 → 100755',
  )
  assert.equal(
    reviewFileDiffEmptyLabel(emptyDiff({ kind: 'Added', old_mode: null })),
    'Empty file added',
  )
  assert.equal(
    reviewFileDiffEmptyLabel(emptyDiff({ kind: 'Deleted', new_mode: null })),
    'Empty file deleted',
  )
})
