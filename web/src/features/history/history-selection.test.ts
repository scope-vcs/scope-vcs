import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntryDetail } from '@/api/types'
import { historyFileSelection, historySelectedFilePath } from './history-selection'

const files = [{ path: '/first.ts' }, { path: '/second.ts' }]

test('history selects the first available file only when the URL has no path', () => {
  assert.equal(historySelectedFilePath(undefined, undefined, false), null)
  assert.equal(historySelectedFilePath(undefined, [], false), null)
  assert.equal(historySelectedFilePath(undefined, files, false), '/first.ts')
  assert.equal(historySelectedFilePath('/second.ts', files, false), '/second.ts')
  assert.equal(historySelectedFilePath('/missing.ts', files, false), '/missing.ts')
})

test('closing a diff dismisses only the current location and explicit selection can reopen it', () => {
  assert.equal(historySelectedFilePath('/second.ts', files, true), null)
  assert.equal(historySelectedFilePath(undefined, files, true), null)
  assert.equal(historySelectedFilePath('/second.ts', files, false), '/second.ts')
})

test('selects exact visibility effects independently of content and same-path transitions', () => {
  const content: HistoryEntryDetail['files'][number] = { path: '/same.ts', kind: 'Modified', old_mode: '100644', new_mode: '100644', old_oid: 'a', new_oid: 'b', visibility: 'Public' }
  const preview = { ...content, old_oid: null, new_oid: 'c' }
  const detail: Pick<HistoryEntryDetail, 'files' | 'visibility_changes'> = {
    files: [content],
    visibility_changes: [
      { id: 'first', path: '/same.ts', old_visibility: 'Private', new_visibility: 'Public', file: preview },
      { id: 'second', path: '/same.ts', old_visibility: 'Public', new_visibility: 'Private', file: null },
    ],
  }
  assert.equal(historyFileSelection({ path: '/same.ts' }, detail, false).file, content)
  assert.equal(historyFileSelection({ path: '/same.ts', visibility_change: 'first' }, detail, false).file, preview)
  assert.equal(historyFileSelection({ path: '/same.ts', visibility_change: 'second' }, detail, false).file, null)
  assert.equal(historyFileSelection({ path: '/other.ts', visibility_change: 'first' }, detail, false).file, null)
  assert.equal(historyFileSelection({ visibility_change: 'first' }, detail, false).path, '/same.ts')
  assert.deepEqual(historyFileSelection({ visibility_change: 'first' }, detail, true), { path: null, file: null, visibilityId: null })
})

test('visibility-only details do not open a content diff by default', () => {
  const detail = { files: [], visibility_changes: [] }
  assert.equal(historyFileSelection({}, detail, false).path, null)
})
