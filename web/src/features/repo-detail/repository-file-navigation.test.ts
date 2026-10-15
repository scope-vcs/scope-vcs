import assert from 'node:assert/strict'
import test from 'node:test'
import { matchingRepositoryFiles, REPOSITORY_FILE_MATCH_LIMIT } from './repository-file-navigation'

const files = [
  { path: '/README.md' },
  { path: '/src/app.ts' },
  { path: '/docs/install.md' },
  { path: '/LICENSE' },
  { path: '/.github/CONTRIBUTING.md' },
  { path: '/docs/SECURITY.md' },
]

test('finds visible files by filename or nested path regardless of case', () => {
  assert.deepEqual(matchingRepositoryFiles(files, ' APP.ts '), { shown: [{ path: '/src/app.ts' }], total: 1 })
  assert.deepEqual(matchingRepositoryFiles(files, 'docs/in'), { shown: [{ path: '/docs/install.md' }], total: 1 })
  assert.deepEqual(matchingRepositoryFiles(files, 'private/'), { shown: [], total: 0 })
  assert.deepEqual(matchingRepositoryFiles(files, ''), { shown: files, total: files.length })
})

test('shows only the first matches of a broad search while counting all of them', () => {
  const many = Array.from({ length: REPOSITORY_FILE_MATCH_LIMIT + 5 }, (_, index) => ({ path: `/src/file-${index}.ts` }))
  const { shown, total } = matchingRepositoryFiles(many, 'src/')
  assert.deepEqual(shown, many.slice(0, REPOSITORY_FILE_MATCH_LIMIT))
  assert.equal(total, many.length)
})
