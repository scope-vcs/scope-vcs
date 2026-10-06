import assert from 'node:assert/strict'
import test from 'node:test'
import { repoViews } from '../../api/repo-views'
import type { HistoryEntryDetailResponse, NativeHistoryCommitResponse } from '../../api/types.generated'
import { historyNativeCommitRows, historyNativeCommitsHeading } from './history-native-commits-model'

const views = repoViews([
  { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
  { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
  { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
])

test('a narrower history names the view that preserved the request commits', () => {
  assert.equal(historyNativeCommitsHeading({ view: 'agent' }, views), 'Request commits preserved in the Agent view')
  assert.equal(historyNativeCommitsHeading({ view: 'public' }, views), 'Request commits preserved in the Public view')
  assert.equal(historyNativeCommitsHeading({ view: 'private' }, views), 'Request commits')
})

test('request commits list their short id, first message line, author and file count', () => {
  const file = { path: '/src/lib.rs', kind: 'Modified', old_mode: null, new_mode: null, old_oid: null, new_oid: null, label: 'agent' }
  const commit = (oid: string, message: string, files: number): NativeHistoryCommitResponse => ({
    oid,
    parent_oids: [],
    tree_oid: 'f'.repeat(40),
    author: 'ada',
    message,
    occurred_at_unix: 1,
    files: Array.from({ length: files }, () => file) as NativeHistoryCommitResponse['files'],
  })
  const detail = {
    native_commits: [commit('a'.repeat(40), 'Teach the agent\n\nbody', 1), commit('b'.repeat(40), '', 2)],
  } as Pick<HistoryEntryDetailResponse, 'native_commits'>
  assert.deepEqual(historyNativeCommitRows(detail), [
    { oid: 'a'.repeat(40), shortOid: 'aaaaaaaaaaaa', title: 'Teach the agent', author: 'ada', fileCount: '1 file' },
    { oid: 'b'.repeat(40), shortOid: 'bbbbbbbbbbbb', title: '(no message)', author: 'ada', fileCount: '2 files' },
  ])
})
