import assert from 'node:assert/strict'
import test from 'node:test'
import { matchingRepositoryFiles, repositoryResources } from './repository-file-navigation'

const files = [
  { path: '/README.md' },
  { path: '/src/app.ts' },
  { path: '/docs/install.md' },
  { path: '/LICENSE' },
  { path: '/.github/CONTRIBUTING.md' },
  { path: '/docs/SECURITY.md' },
]

test('finds visible files by filename or nested path regardless of case', () => {
  assert.deepEqual(matchingRepositoryFiles(files, ' APP.ts '), [{ path: '/src/app.ts' }])
  assert.deepEqual(matchingRepositoryFiles(files, 'docs/in'), [{ path: '/docs/install.md' }])
  assert.deepEqual(matchingRepositoryFiles(files, 'private/'), [])
  assert.deepEqual(matchingRepositoryFiles(files, ''), files)
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
