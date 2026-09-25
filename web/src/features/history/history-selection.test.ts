import assert from 'node:assert/strict'
import test from 'node:test'
import { historyFileSelection } from './history-selection'
import type { HistoryEntryDetailResponse } from '@/api/types.generated'

const files = [{ path: '/first.ts' }, { path: '/second.ts' }]

function detailWith(entries: { path: string }[]) {
  return { files: entries as HistoryEntryDetailResponse['files'], visibility_changes: [] }
}

test('history opens a file only from the URL path', () => {
  assert.equal(historyFileSelection({}, null).path, null)
  assert.equal(historyFileSelection({}, detailWith([])).path, null)
  assert.equal(historyFileSelection({}, detailWith(files)).path, null)
  assert.equal(historyFileSelection({ path: '/second.ts' }, detailWith(files)).path, '/second.ts')
  assert.equal(historyFileSelection({ path: '/missing.ts' }, detailWith(files)).file, null)
})

test('selects exact visibility effects independently of content and same-path transitions', () => {
  const content: HistoryEntryDetailResponse['files'][number] = { path: '/same.ts', kind: 'Modified', old_mode: '100644', new_mode: '100644', old_oid: 'a', new_oid: 'b', visibility: 'Public' }
  const preview = { ...content, old_oid: null, new_oid: 'c' }
  const detail: Pick<HistoryEntryDetailResponse, 'files' | 'visibility_changes'> = {
    files: [content],
    visibility_changes: [
      { id: 'first', path: '/same.ts', old_visibility: 'Private', new_visibility: 'Public', file: preview },
      { id: 'second', path: '/same.ts', old_visibility: 'Public', new_visibility: 'Private', file: null },
    ],
  }
  assert.equal(historyFileSelection({ path: '/same.ts' }, detail).file, content)
  assert.equal(historyFileSelection({ path: '/same.ts', visibility_change: 'first' }, detail).file, preview)
  assert.equal(historyFileSelection({ path: '/same.ts', visibility_change: 'second' }, detail).file, null)
  assert.equal(historyFileSelection({ path: '/other.ts', visibility_change: 'first' }, detail).file, null)
  assert.equal(historyFileSelection({ visibility_change: 'first' }, detail).path, '/same.ts')
})

test('visibility-only details do not open a content diff by default', () => {
  const detail = { files: [], visibility_changes: [] }
  assert.equal(historyFileSelection({}, detail).path, null)
})
