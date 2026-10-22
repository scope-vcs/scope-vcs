import assert from 'node:assert/strict'
import test from 'node:test'
import { reviewRequestCommentDraft } from './repo-github-review-bots-model'

test('a review request comment saves when its trimmed text changes and fits 500 characters', () => {
  assert.deepEqual(reviewRequestCommentDraft('  @bot review ', '@bot review'), { tooLong: false, changed: false })
  assert.deepEqual(reviewRequestCommentDraft('', '@bot review'), { tooLong: false, changed: true })
  assert.deepEqual(reviewRequestCommentDraft('', null), { tooLong: false, changed: false })
  assert.deepEqual(reviewRequestCommentDraft('é'.repeat(500), null), { tooLong: false, changed: true })
  assert.deepEqual(reviewRequestCommentDraft('é'.repeat(501), null), { tooLong: true, changed: true })
})
