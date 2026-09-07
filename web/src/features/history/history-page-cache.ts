import type { HistoryPage } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'
import type { LoadedHistory } from './history-pagination'

const entries = createBoundedCache<string, LoadedHistory>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function historyPageCacheKey(scope: string, page: HistoryPage) {
  return JSON.stringify([scope, page.repo_id, page.generation, page.view_key, page.audience, page.feed])
}

export function restoreHistoryPages(key: string | null, initialPage: HistoryPage): LoadedHistory {
  return (key ? entries.get(key) : undefined) ?? {
    entries: initialPage.entries,
    next_cursor: initialPage.next_cursor,
  }
}

export function retainHistoryPages(key: string | null, loaded: LoadedHistory) {
  if (key) entries.set(key, loaded)
}

export function resetHistoryPageCache() {
  entries.clear()
}
