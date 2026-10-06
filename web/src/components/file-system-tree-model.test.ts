import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  ancestorFolderKeys,
  buildFileSystemTree,
  folderVisibility,
  normalizeFilePath,
} from './file-system-tree-model'

type TestFile = { path: string; label: 'private' | 'public' }

test('file tree normalizes, nests, sorts, and summarizes paths', () => {
  const tree = buildFileSystemTree<TestFile>([
    { path: '/src/zeta.ts', label: 'public' },
    { path: 'README.md', label: 'public' },
    { path: String.raw`src\components\Button.tsx`, label: 'private' },
    { path: './docs//guide.md', label: 'public' },
    { path: '/src/components/Alert.tsx', label: 'private' },
  ])

  assert.deepEqual(tree.children.map(({ type, name }) => [type, name]), [
    ['folder', 'docs'], ['folder', 'src'], ['file', 'README.md'],
  ])
  const src = tree.children[1]
  assert.equal(src.type, 'folder')
  assert.deepEqual(src.children.map(({ type, name }) => [type, name]), [
    ['folder', 'components'], ['file', 'zeta.ts'],
  ])
  assert.deepEqual(src.files.map(({ path }) => normalizeFilePath(path)), [
    'src/components/Alert.tsx', 'src/components/Button.tsx', 'src/zeta.ts',
  ])
  assert.deepEqual(ancestorFolderKeys('/src/components/Button.tsx'), [
    'folder:/src',
    'folder:/src/components',
  ])
  assert.equal(folderVisibility(src.files), 'Mixed')
  assert.equal(folderVisibility([{ path: 'a', label: 'public' }]), 'public')
  assert.equal(folderVisibility([{ path: 'a', label: 'private' }]), 'private')
  assert.equal(
    normalizeFilePath(String.raw`.\src\\..\components/Button.tsx`),
    'src/components/Button.tsx',
  )
})
