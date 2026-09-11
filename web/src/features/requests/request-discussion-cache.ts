import { createCachedResource } from '../../lib/cached-resource'
import { resourceErrorMessage } from '../../lib/use-cached-resource'
import { collectionFromPage, mergeRefreshedDiscussionPage, type DiscussionCollection } from './request-discussion-model'
import { createRequestDiscussionSync } from './request-discussion-sync'
import type { RequestDiscussionChanges, RequestDiscussionPage } from './request-discussion-types'

type DiscussionSession = {
  collection: DiscussionCollection
  dataGeneration: number
  error: string | null
  loadingMore: boolean
  scrollTop: number
  sync: ReturnType<typeof createRequestDiscussionSync>
  refresh: (page: RequestDiscussionPage) => Promise<void>
  updateCollection: (update: (current: DiscussionCollection) => DiscussionCollection, dataChanged?: boolean) => void
  setError: (error: string | null) => void
  setLoadingMore: (loadingMore: boolean) => void
}

export const requestDiscussionResource = createCachedResource<DiscussionSession>({
  maxEntries: 8,
  maxWeight: 8 * 500,
  weightOf: ({ collection }) => collection.order.length,
})

export function requestDiscussionCacheKey({ repoId, requestId, viewerId }: {
  repoId: string
  requestId: string
  viewerId: string
}) {
  return [viewerId, repoId, requestId].join('\0')
}

export function openRequestDiscussion(
  key: string,
  page: RequestDiscussionPage,
  loadChanges: (after: number) => Promise<RequestDiscussionChanges>,
): DiscussionSession {
  const cached = requestDiscussionResource.peek(key)
  if (cached) return cached

  const read = () => requestDiscussionResource.peek(key) ?? session
  const update = (patch: Partial<DiscussionSession>) => {
    // A late operation must not resurrect an evicted or replaced session.
    if (requestDiscussionResource.peek(key)?.sync !== sync) return
    requestDiscussionResource.write(key, { ...read(), ...patch })
  }
  const updateCollection: DiscussionSession['updateCollection'] = (transform, dataChanged = true) => {
    const current = read()
    const collection = transform(current.collection)
    if (collection !== current.collection) {
      update({ collection, dataGeneration: current.dataGeneration + Number(dataChanged) })
    }
  }
  const sync = createRequestDiscussionSync({
    getCollection: () => read().collection,
    getDataGeneration: () => read().dataGeneration,
    loadChanges: (after) => {
      if (requestDiscussionResource.peek(key)?.sync !== sync) {
        sync.stop()
        return Promise.resolve({ discussions: [], through_position: after, has_more: false })
      }
      return loadChanges(after)
    },
    onCatchUpError: (error) => update({
      error: resourceErrorMessage(error, 'New discussion activity could not be loaded.'),
    }),
    setCollection: (collection) => updateCollection(() => collection),
  })
  const session: DiscussionSession = {
    collection: collectionFromPage(page),
    dataGeneration: 0,
    error: null,
    loadingMore: false,
    scrollTop: 0,
    sync,
    updateCollection,
    refresh: async (incoming) => {
      if (incoming.snapshot_version > read().collection.snapshotVersion) {
        await sync.refresh(() => Promise.resolve(incoming))
      } else {
        // A reopened first page or focused row must not discard loaded older pages.
        updateCollection((current) => mergeRefreshedDiscussionPage(current, incoming, false))
      }
      await sync.catchUp()
    },
    setError: (error) => update({ error }),
    setLoadingMore: (loadingMore) => update({ loadingMore }),
  }
  requestDiscussionResource.write(key, session)
  sync.reset(key)
  return session
}

export function readRequestDiscussionScroll(key: string) {
  return requestDiscussionResource.peek(key)?.scrollTop ?? 0
}

export function writeRequestDiscussionScroll(key: string, scrollTop: number) {
  const session = requestDiscussionResource.peek(key)
  if (session) requestDiscussionResource.write(key, { ...session, scrollTop })
}

export function resetRequestDiscussionCache() {
  requestDiscussionResource.clear()
}
