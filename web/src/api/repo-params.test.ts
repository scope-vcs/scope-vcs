import assert from 'node:assert/strict'
import test from 'node:test'
import { parseRepoParams } from './repo-params'

test('parseRepoParams trims segments and discards extra fields', () => {
  assert.deepEqual(
    parseRepoParams({ owner: ' scope ', repo: 'vcs', extra: 'discard' }),
    { owner: 'scope', repo: 'vcs' },
  )
})

test('parseRepoParams rejects missing, empty, and multi-segment values', () => {
  for (const input of [null, undefined, {}, { owner: 'scope' }, { owner: 'scope', repo: ' ' }, { owner: 42, repo: 'vcs' }]) {
    assert.throws(() => parseRepoParams(input), /incomplete/)
  }
  assert.throws(() => parseRepoParams({ owner: 'scope/other', repo: 'vcs' }), /single path segments/)
  assert.throws(() => parseRepoParams({ owner: 'scope', repo: 'vcs/nested' }), /single path segments/)
})
