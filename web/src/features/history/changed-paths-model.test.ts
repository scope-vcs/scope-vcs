import assert from 'node:assert/strict'
import test from 'node:test'
import type { CommitFileResponse, FileChangeKind } from '@/api/types.generated'
import {
  changedPathFilterCounts,
  changedPaths,
  changedPathTargets,
  filterChangedPaths,
  groupChangedPaths,
  initiallyOpenGroups,
  openableChangedPaths,
  steppedChangedPath,
  visibilitySummary,
  type HistoryVisibilityChange,
} from './changed-paths-model'

function file(path: string, kind: FileChangeKind = 'Modified'): CommitFileResponse {
  return { path, kind, old_mode: null, new_mode: null, old_oid: 'a', new_oid: 'b', label: 'private' }
}

function change(id: string, path: string, preview: boolean, new_label = 'public'): HistoryVisibilityChange {
  return { id, path, old_label: 'private', new_label, file: preview ? file(path, 'Added') : null }
}

test('one row per path merges code and visibility changes in display order', () => {
  const code = file('/web/src/b.ts')
  const toPublic = change('set:b', '/web/src/b.ts', true)
  const back = change('set2:b', '/web/src/b.ts', false, 'private')
  const paths = changedPaths(
    [code, file('/api/z.rs', 'Added'), file('/README.md'), file('/web/a.ts', 'Deleted')],
    [toPublic, back, change('set:c', '/api/lib/c.rs', true)],
  )

  assert.deepEqual(paths.map((path) => path.path), [
    'README.md', 'api/z.rs', 'api/lib/c.rs', 'web/a.ts', 'web/src/b.ts',
  ])
  const merged = paths.at(-1)!
  assert.equal(merged.file, code)
  assert.deepEqual(merged.visibility, [toPublic, back])
  assert.deepEqual(changedPathTargets(merged), [
    { kind: 'file', file: code },
    { kind: 'visibility', change: toPublic },
  ])

  const { root, groups } = groupChangedPaths(paths)
  assert.deepEqual(root.map((path) => path.path), ['README.md'])
  assert.deepEqual(groups.map((group) => [group.name, group.paths.length]), [['api', 2], ['web', 2]])
})

test('filters count and select paths by visibility, change kind, and query', () => {
  const paths = changedPaths(
    [file('/api/a.rs', 'Added'), file('/api/b.rs'), file('/web/c.ts')],
    [change('set:a', '/api/a.rs', true), change('set:d', '/docs/d.md', false)],
  )

  assert.deepEqual(changedPathFilterCounts(paths), [
    { filter: 'all', count: 4 },
    { filter: 'visibility', count: 2 },
    { filter: 'Added', count: 1 },
    { filter: 'Modified', count: 2 },
  ])
  assert.deepEqual(filterChangedPaths(paths, 'visibility', '').map((path) => path.path), ['api/a.rs', 'docs/d.md'])
  assert.deepEqual(filterChangedPaths(paths, 'Modified', ' API/ ').map((path) => path.path), ['api/b.rs'])
  assert.deepEqual(visibilitySummary(paths), { count: 2, label: 'public' })
  assert.equal(visibilitySummary(changedPaths([file('/a.ts')], [])), null)
  assert.deepEqual(
    visibilitySummary(changedPaths([], [change('x', '/a', true), change('y', '/a', false, 'private')])),
    { count: 1, label: 'private' },
  )
  assert.deepEqual(
    visibilitySummary(changedPaths([], [change('x', '/a', false), change('y', '/b', false, 'agent')])),
    { count: 2, label: 'Mixed' },
  )
})

test('stepping moves through openable paths and stops at either end', () => {
  const paths = changedPaths(
    [file('/a/1.ts'), file('/a/3.ts')],
    [change('set:2', '/a/2.ts', false), change('set:4', '/a/4.ts', true)],
  )
  const openable = openableChangedPaths(paths)
  const step = (current: number, direction: 1 | -1) =>
    steppedChangedPath(openable, current, direction)?.path

  assert.deepEqual(openable.map((path) => path.path), ['a/1.ts', 'a/3.ts', 'a/4.ts'])
  assert.equal(step(-1, 1), 'a/1.ts')
  assert.equal(step(-1, -1), 'a/4.ts')
  assert.equal(step(0, 1), 'a/3.ts')
  assert.equal(step(2, 1), 'a/4.ts')
  assert.equal(step(0, -1), 'a/1.ts')
  assert.equal(steppedChangedPath(openableChangedPaths(changedPaths([], [change('x', '/a', false)])), -1, 1), null)
})

test('small changes open every folder and large ones open only the selected folder', () => {
  const small = changedPaths([file('/api/a.rs'), file('/web/b.ts')])
  assert.deepEqual([...initiallyOpenGroups(small, null)], ['api', 'web'])

  const large = changedPaths(Array.from({ length: 21 }, (_, index) => file(`/${index % 3}/f${index}.ts`)))
  assert.deepEqual([...initiallyOpenGroups(large, null)], [])
  assert.deepEqual([...initiallyOpenGroups(large, '/1/f4.ts')], ['1'])

  const singleGroup = changedPaths(Array.from({ length: 21 }, (_, index) => file(`/web/f${index}.ts`)))
  assert.deepEqual([...initiallyOpenGroups(singleGroup, null)], [])
  assert.deepEqual([...initiallyOpenGroups(singleGroup, '/web/f4.ts')], ['web'])
})
