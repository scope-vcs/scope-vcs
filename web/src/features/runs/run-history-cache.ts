import type { RepoRunHistoryInput } from '@/api/types'
import type { RepositoryRunHistoryPageResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { mergeRunHistory, reloadRunHistoryPages } from './run-history-model'

type RetainedRunHistory = {
  history: RepositoryRunHistoryPageResponse | null
  snapshot: RepositoryRunHistoryPageResponse | null
  pageCount: number
}

export const runHistoryResource = createCachedResource<RetainedRunHistory>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function runHistoryCacheKey(scope: string, workflow?: string) {
  return JSON.stringify([scope, workflow ?? null])
}

export function restoreRunHistory(key: string | null, initial: RepositoryRunHistoryPageResponse | null): RetainedRunHistory {
  if (!initial) return { history: null, snapshot: null, pageCount: 1 }
  const cached = key ? runHistoryResource.read(key) : undefined
  if (cached?.history && JSON.stringify(cached.snapshot) === JSON.stringify(initial)) return cached
  if (!cached?.history || cached.pageCount === 1) return { history: initial, snapshot: initial, pageCount: 1 }
  return {
    history: {
      runs: mergeRunHistory(initial.runs, cached.history.runs),
      next_cursor: cached.history.next_cursor,
    },
    snapshot: initial,
    pageCount: cached.pageCount,
  }
}

export function initializeRunHistory(key: string, initial: RepositoryRunHistoryPageResponse | null) {
  const retained = restoreRunHistory(key, initial)
  if (retained !== runHistoryResource.peek(key)) runHistoryResource.write(key, retained)
}

type HistoryRequest = {
  key: string
  input: RepoRunHistoryInput
  loadHistory: (input: RepoRunHistoryInput, signal?: AbortSignal) => Promise<RepositoryRunHistoryPageResponse | null>
}

export async function refreshRunHistory({ key, input, loadHistory }: HistoryRequest): Promise<void> {
  const snapshot = runHistoryResource.getSnapshot(key)
  const current = snapshot.value
  if (!current?.history) return
  const load = async (signal: AbortSignal) => {
    const history = await reloadRunHistoryPages(current.pageCount,
      (after) => loadHistory({ ...input, after }, AbortSignal.any([signal, AbortSignal.timeout(15_000)])))
    return { ...current, history }
  }
  if (snapshot.pending) {
    if (snapshot.version === 'refresh') {
      await runHistoryResource.load(key, 'refresh', load)
      return
    }
    // A live change during pagination must reconcile the newly loaded depth.
    await runHistoryResource.ensure(key, snapshot.version!, load)
    return refreshRunHistory({ key, input, loadHistory })
  }
  runHistoryResource.invalidate(key)
  await runHistoryResource.load(key, 'refresh', load)
}

export async function loadMoreRunHistory({ key, input, loadHistory }: HistoryRequest): Promise<void> {
  const snapshot = runHistoryResource.getSnapshot(key)
  const current = snapshot.value
  if (snapshot.pending || !current?.history?.next_cursor) return
  const after = current.history.next_cursor
  runHistoryResource.invalidate(key)
  await runHistoryResource.ensure(key, 'more', async (signal) => {
    const next = await loadHistory({ ...input, after }, AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
    return {
      ...current,
      history: next ? {
        next_cursor: next.next_cursor,
        runs: mergeRunHistory(current.history!.runs, next.runs),
      } : null,
      pageCount: current.pageCount + (next ? 1 : 0),
    }
  })
}
