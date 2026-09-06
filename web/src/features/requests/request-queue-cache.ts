import { createBoundedCache } from '../../lib/bounded-cache'
import {
  createRequestQueueViewState,
  requestQueueViewReducer,
  type RequestQueuePages,
  type RequestQueueViewState,
} from './request-list-model'

const entries = createBoundedCache<string, RequestQueueViewState>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function restoreRequestQueue(key: string | null, pages: RequestQueuePages) {
  const cached = key ? entries.get(key) : undefined
  return cached
    ? requestQueueViewReducer(cached, { type: 'loader_snapshot_received', pages })
    : createRequestQueueViewState(pages)
}

export function retainRequestQueue(key: string | null, state: RequestQueueViewState) {
  if (!key) return
  entries.set(key, {
    ...state,
    loadingSection: null,
    searching: false,
    searchError: null,
    sectionErrors: {},
  })
}

export function resetRequestQueueCache() {
  entries.clear()
}
