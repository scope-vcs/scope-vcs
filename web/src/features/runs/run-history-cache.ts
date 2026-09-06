import type { RepoRunHistoryPage } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'
import { mergeRunHistory } from './run-history-model'

export type RetainedRunHistory = {
  history: RepoRunHistoryPage | null
  snapshot: RepoRunHistoryPage | null
  pageCount: number
}

const entries = createBoundedCache<string, RetainedRunHistory>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function runHistoryCacheKey(scope: string, workflow?: string) {
  return JSON.stringify([scope, workflow ?? null])
}

export function restoreRunHistory(key: string | null, initial: RepoRunHistoryPage | null): RetainedRunHistory {
  if (!initial) return { history: null, snapshot: null, pageCount: 1 }
  const cached = key ? entries.get(key) : undefined
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

export function retainRunHistory(key: string | null, value: RetainedRunHistory) {
  if (key) entries.set(key, value)
}

export function resetRunHistoryCache() {
  entries.clear()
}
