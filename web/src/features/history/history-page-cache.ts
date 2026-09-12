import { createBoundedCache } from '../../lib/bounded-cache'
import type { LoadedHistory } from './history-pagination'
import type { HistoryPageResponse } from '@/api/types.generated'

const entries = createBoundedCache<string, LoadedHistory>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function historyPageCacheKey(scope: string, page: HistoryPageResponse) {
  return JSON.stringify([scope, page.repo_id, page.generation, page.view_key, page.audience, page.feed])
}

export function restoreHistoryPages(key: string | null, initialPage: HistoryPageResponse): LoadedHistory {
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
