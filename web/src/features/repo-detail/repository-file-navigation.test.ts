import assert from 'node:assert/strict'
import test from 'node:test'
import {
  matchingRepositoryFiles,
  REPOSITORY_FILE_MATCH_LIMIT,
  repositoryResources,
} from './repository-file-navigation'

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

test('offers only resources present in the visible file list', () => {
  assert.deepEqual(repositoryResources(files), [
    { label: 'License', path: '/LICENSE' },
    { label: 'Contributing', path: '/.github/CONTRIBUTING.md' },
    { label: 'Security policy', path: '/docs/SECURITY.md' },
  ])
  assert.deepEqual(repositoryResources(files.slice(0, 3)), [])
  assert.deepEqual(repositoryResources([]), [])
})

test('prefers root resources over secondary locations and excludes unrelated filenames', () => {
  assert.deepEqual(repositoryResources([
    { path: '/vendor/LICENSE' },
    { path: '/docs/SECURITY.md' },
    { path: '/SECURITY.md' },
    { path: '/src/security.ts' },
    { path: '/LICENCE.txt' },
  ]), [
    { label: 'License', path: '/LICENCE.txt' },
    { label: 'Security policy', path: '/SECURITY.md' },
  ])
})
