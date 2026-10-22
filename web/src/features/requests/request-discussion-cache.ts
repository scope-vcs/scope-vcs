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
  syncProjections: (requestHead: string, load: LoadProjectionPage) => Promise<void>
  updateCollection: (update: (current: DiscussionCollection) => DiscussionCollection, dataChanged?: boolean) => void
  setError: (error: string | null) => void
  setLoadingMore: (loadingMore: boolean) => void
}

const DISCUSSION_PAGE_MAX = 100

type LoadProjectionPage = (input: { cursor?: string; limit: number }) => Promise<RequestDiscussionPage>

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

  const session = createDiscussionSession(key, page, loadChanges)
  requestDiscussionResource.write(key, session)
  return session
}

function createDiscussionSession(
  key: string,
  page: RequestDiscussionPage,
  loadChanges: (after: number) => Promise<RequestDiscussionChanges>,
): DiscussionSession {
  const read = () => requestDiscussionResource.peek(key) ?? session
  const update = (patch: Partial<DiscussionSession>) => {
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
  let projectedHead: string | null = null
  let requestedHead: string | null = null
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
        updateCollection((current) => mergeRefreshedDiscussionPage(current, incoming, false))
      }
      await sync.catchUp()
    },
    syncProjections: async (requestHead, load) => {
      if (requestHead === projectedHead || requestHead === requestedHead) return
      const retained = read().collection
      if (projectedHead === null || !retained.order.some((id) => retained.byId.get(id)?.anchor?.lines)) {
        projectedHead = requestHead
        return
      }
      requestedHead = requestHead
      const current = () => requestedHead === requestHead && requestDiscussionResource.peek(key)?.sync === sync
      try {
        const pages: RequestDiscussionPage[] = []
        let cursor: string | undefined
        let covered = 0
        do {
          const page = await load({ cursor, limit: DISCUSSION_PAGE_MAX })
          if (!current()) return
          pages.push(page)
          covered += page.discussions.length
          cursor = page.next_cursor ?? undefined
        } while (cursor && covered < retained.order.length)
        updateCollection((collection) => pages.reduce(
          (next, page) => mergeRefreshedDiscussionPage(next, page, false),
          collection,
        ))
        projectedHead = requestHead
      } catch (error) {
        if (current()) update({ error: resourceErrorMessage(error, 'Discussion positions could not be refreshed.') })
      } finally {
        if (requestedHead === requestHead) requestedHead = null
      }
    },
    setError: (error) => update({ error }),
    setLoadingMore: (loadingMore) => update({ loadingMore }),
  }
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

function discussionSessionPage(session: DiscussionSession): RequestDiscussionPage {
  const { collection } = session
  return {
    discussions: collection.order.flatMap((id) => {
      const discussion = collection.byId.get(id)
      return discussion ? [discussion] : []
    }).reverse(),
    next_cursor: collection.nextCursor,
    snapshot_version: collection.snapshotVersion,
  }
}

export function loadRequestDiscussionSession({
  key,
  focusedDiscussionId,
  load,
  loadChanges,
}: {
  key: string
  focusedDiscussionId?: string
  load: () => Promise<RequestDiscussionPage | null>
  loadChanges: (after: number) => Promise<RequestDiscussionChanges>
}): RequestDiscussionPage | null | Promise<RequestDiscussionPage | null> {
  const cached = requestDiscussionResource.peek(key)
  const retained = cached ? discussionSessionPage(cached) : null
  if (retained && (!focusedDiscussionId || retained.discussions.some(({ id }) => id === focusedDiscussionId))) return retained
  if (cached && retained) return load().then((incoming) => {
    if (requestDiscussionResource.peek(key)?.sync !== cached.sync) throw new Error('Resource is no longer available.')
    if (incoming) cached.updateCollection((current) => mergeRefreshedDiscussionPage(current, incoming, false))
    const current = requestDiscussionResource.peek(key)
    if (current?.sync !== cached.sync) throw new Error('Resource is no longer available.')
    return discussionSessionPage(current)
  }, () => {
    const current = requestDiscussionResource.peek(key)
    if (current?.sync !== cached.sync) throw new Error('Resource is no longer available.')
    return discussionSessionPage(current)
  })
  return requestDiscussionResource.load(key, '', async () => {
    const page = await load()
    if (!page) throw new Error('Discussion is unavailable.')
    return createDiscussionSession(key, page, loadChanges)
  }).then((session) => {
    const page = discussionSessionPage(session)
    if (!focusedDiscussionId || page.discussions.some(({ id }) => id === focusedDiscussionId)) return page
    return loadRequestDiscussionSession({ key, focusedDiscussionId, load, loadChanges })
  })
}
