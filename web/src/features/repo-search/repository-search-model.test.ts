import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummaryResponse, RequestQueueItemResponse } from '../../api/types.generated'
import { repositorySearchFeed, repositorySearchGroups, type RepositorySearchGroup, type RepositorySearchFilter } from './repository-search-model'

const request = { request: { id: 'req_1', title: 'Retry runners' } } as RequestQueueItemResponse
const push = { kind: 'push', source_id: 'push_1' } as HistoryEntrySummaryResponse
const merge = { kind: 'merged_request', source_id: 'merge_1' } as HistoryEntrySummaryResponse
const visibility = { kind: 'visibility_change', source_id: 'vis_1' } as HistoryEntrySummaryResponse
const views = { kind: 'views_change', source_id: 'views_1' } as HistoryEntrySummaryResponse
const files = [
  { label: 'public', oid: 'a', path: '/runner/Dockerfile', tracked: true },
  { label: 'public', oid: 'b', path: '/README.md', tracked: true },
] as const

function groups(filter: RepositorySearchFilter, query: string) {
  return repositorySearchGroups({
    files: [...files],
    filter,
    query,
    results: { history: [push, merge, visibility, views], historyCursor: null, requestCursors: {}, requests: [request] },
    version: { kind: 'main' },
  })
}

function rowIds(group: RepositorySearchGroup | undefined) {
  return group?.rows.map((row) => row.id)
}

test('an empty search lists versions and history, and leaves files for the Files filter', () => {
  const [versions, history, extra] = groups('all', '')
  assert.deepEqual(rowIds(versions), ['main', 'request:req_1'])
  assert.equal(versions?.label, 'Versions')
  assert.equal(history?.rows.length, 4)
  assert.equal(extra, undefined)
})

test('a query groups matches by kind and drops main unless it matches', () => {
  const [requests, history, matched] = groups('all', 'runner')
  assert.deepEqual(rowIds(requests), ['request:req_1'])
  assert.equal(requests?.label, 'Requests')
  assert.equal(history?.id, 'history')
  assert.deepEqual(rowIds(matched), ['file:/runner/Dockerfile'])
  assert.equal(matched?.label, 'Files on main')
  assert.deepEqual(rowIds(groups('all', 'mai')[0]), ['main', 'request:req_1'])
})

test('history filters ask the server for their feed, so older matches are not cut off', () => {
  assert.equal(repositorySearchFeed('updates'), 'updates')
  assert.equal(repositorySearchFeed('visibility'), 'visibility')
  assert.equal(repositorySearchFeed('requests'), 'all')
})

test('the Requests filter shows every loaded request so paging can reach later matches', () => {
  const many = Array.from({ length: 40 }, (_, index) => ({ request: { id: `req_${index}` } }) as RequestQueueItemResponse)
  const [requests] = repositorySearchGroups({
    files: [],
    filter: 'requests',
    query: 'req',
    results: { history: [], historyCursor: null, requestCursors: {}, requests: many },
    version: { kind: 'main' },
  })
  assert.equal(requests?.rows.length, 40)
})

test('the Files filter lists files without a query', () => {
  assert.deepEqual(rowIds(groups('files', '')[0]), ['file:/runner/Dockerfile', 'file:/README.md'])
})
