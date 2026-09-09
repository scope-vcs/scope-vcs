import { createCachedResource } from '../../lib/cached-resource'
import {
  createRequestQueueViewState,
  requestQueueViewReducer,
  type RequestQueuePages,
  type RequestQueueViewAction,
  type RequestQueueViewState,
} from './request-list-model'

export const requestQueueResource = createCachedResource<RequestQueueViewState & { owner: object }>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function openRequestQueue(key: string, pages: RequestQueuePages) {
  const cached = requestQueueResource.peek(key)
  if (cached?.snapshot === pages) return cached
  const state = cached
    ? requestQueueViewReducer(cached, { type: 'loader_snapshot_received', pages })
    : createRequestQueueViewState(pages)
  const session = { ...state, owner: cached?.owner ?? {} }
  requestQueueResource.write(key, session)
  return session
}

export function dispatchRequestQueue(key: string, action: RequestQueueViewAction, owner: object) {
  const state = requestQueueResource.peek(key)
  if (!state || state.owner !== owner) return
  const next = requestQueueViewReducer(state, action)
  if (next !== state) requestQueueResource.write(key, { ...next, owner })
}

export function resetRequestQueueCache() {
  requestQueueResource.clear()
}
