import assert from 'node:assert/strict'
import test from 'node:test'
import { runHistoryResource, runHistoryCacheKey } from './run-history-cache'
import type { RepositoryRunHistoryPageResponse } from '@/api/types.generated'

function page(ids: string[], next_cursor: string | null = null): RepositoryRunHistoryPageResponse {
  return { runs: ids.map((id) => ({ id, state: 'queued' })) as RepositoryRunHistoryPageResponse['runs'], next_cursor }
}

test('run retention isolates repository access and workflow filters', () => {
  runHistoryResource.clear()
  const key = runHistoryCacheKey('viewer/repo/access')
  runHistoryResource.write(key, { history: page(['first', 'older']), pageCount: 2 })
  for (const other of [runHistoryCacheKey('other-viewer/repo/access'), runHistoryCacheKey('viewer/other-repo/access'), runHistoryCacheKey('viewer/repo/access', 'checks')]) {
    assert.equal(runHistoryResource.read(other), null)
  }
})

test('run pagination retention is bounded', () => {
  runHistoryResource.clear()
  for (let index = 0; index < 13; index++) runHistoryResource.write(String(index), { history: page(['first', 'older']), pageCount: 2 })
  assert.equal(runHistoryResource.read('0'), null)
  assert.equal(runHistoryResource.read('12')?.pageCount, 2)
  runHistoryResource.write('large', { history: page(['x'.repeat(3 * 1024 * 1024)]), pageCount: 2 })
  assert.equal(runHistoryResource.read('large'), null)
})
