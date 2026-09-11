import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoRunHistoryPage } from '@/api/types'
import { restoreRunHistory, runHistoryResource, resetRunHistoryCache, runHistoryCacheKey } from './run-history-cache'
import { reloadRunHistoryPages } from './run-history-model'

function page(ids: string[], next_cursor: string | null = null): RepoRunHistoryPage {
  return { runs: ids.map((id) => ({ id, state: 'queued' })) as RepoRunHistoryPage['runs'], next_cursor }
}

test('navigation restores older runs, exhausted cursor and refresh depth', async () => {
  resetRunHistoryCache()
  const key = runHistoryCacheKey('viewer/repo/access')
  runHistoryResource.write(key, { history: page(['first', 'older']), snapshot: page(['first'], 'older-page'), pageCount: 2 })
  const restored = restoreRunHistory(key, page(['first'], 'older-page'))
  assert.deepEqual(restored.history?.runs.map(({ id }) => id), ['first', 'older'])
  assert.equal(restored.history?.next_cursor, null)
  const cursors: Array<string | undefined> = []
  const refreshed = await reloadRunHistoryPages(restored.pageCount, async (after) => {
    cursors.push(after)
    return after ? page(['changed-older']) : page(['new-first'], 'next')
  })
  assert.deepEqual(cursors, [undefined, 'next'])
  assert.deepEqual(refreshed?.runs.map(({ id }) => id), ['new-first', 'changed-older'])
})

test('authoritative first page updates retained rows before background reconciliation', () => {
  resetRunHistoryCache()
  runHistoryResource.write('runs', { history: page(['first', 'older']), snapshot: page(['first'], 'older-page'), pageCount: 2 })
  const initial = page(['new', 'first'], 'next')
  initial.runs[1]!.state = 'succeeded'
  const restored = restoreRunHistory('runs', initial)
  assert.deepEqual(restored.history?.runs.map(({ id }) => id), ['new', 'first', 'older'])
  assert.equal(restored.history?.runs[1]?.state, 'succeeded')
  assert.equal(restoreRunHistory('runs', null).history, null)
})

test('run retention isolates repository access and workflow filters', () => {
  resetRunHistoryCache()
  const key = runHistoryCacheKey('viewer/repo/access')
  runHistoryResource.write(key, { history: page(['first', 'older']), snapshot: page(['first'], 'older-page'), pageCount: 2 })
  for (const other of [runHistoryCacheKey('other-viewer/repo/access'), runHistoryCacheKey('viewer/other-repo/access'), runHistoryCacheKey('viewer/repo/access', 'checks'), null]) {
    assert.equal(restoreRunHistory(other, page(['first'])).history?.runs.length, 1)
  }
})

test('an unchanged route snapshot does not roll back newer live run state', () => {
  resetRunHistoryCache()
  const snapshot = page(['first'])
  const refreshed = page(['first'])
  refreshed.runs[0]!.state = 'succeeded'
  runHistoryResource.write('live', { history: refreshed, snapshot, pageCount: 1 })
  assert.equal(restoreRunHistory('live', page(['first'])).history?.runs[0]?.state, 'succeeded')
})
