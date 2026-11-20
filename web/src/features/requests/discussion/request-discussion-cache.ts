import { createRepoResource } from '../../repo-detail/repo-resource'
import { resourceErrorMessage } from '../../../lib/use-cached-resource'
import { collectionFromPage, mergeRefreshedDiscussionPage, type DiscussionCollection } from './request-discussion-model'
import type { RequestDiscussionPage } from './request-discussion-types'

export type RequestDiscussionState = {
  caughtUp: boolean
  collection: DiscussionCollection
  epoch: number
  error: string | null
  loadingMore: boolean
  projectedHead: string | null
  scrollTop: number
}

const DISCUSSION_PAGE_MAX = 100

type LoadProjectionPage = (input: { cursor?: string; limit: number }) => Promise<RequestDiscussionPage>

export const requestDiscussionResource = createRepoResource<RequestDiscussionState>({
  invalidatedBy: [],
  maxEntries: 8,
  maxWeight: 8 * 500,
  weightOf: ({ collection }) => collection.order.length,
})

let epochs = 0
const projectionRequests = new Map<string, { epoch: number; head: string }>()

export function requestDiscussionCacheKey({ repoId, requestId, viewerId }: {
  repoId: string
  requestId: string
  viewerId: string
}) {
  return [viewerId, repoId, requestId].join('\0')
}

export function requestDiscussionState(page: RequestDiscussionPage): RequestDiscussionState {
  epochs += 1
  return {
    caughtUp: true,
    collection: collectionFromPage(page),
    epoch: epochs,
    error: null,
    loadingMore: false,
    projectedHead: null,
    scrollTop: 0,
  }
}

export async function loadRequestDiscussionState(load: () => Promise<RequestDiscussionPage | null>) {
  const page = await load()
  if (!page) throw new Error('Discussion is unavailable.')
  return requestDiscussionState(page)
}

export function updateRequestDiscussion(
  key: string,
  epoch: number,
  patch: Partial<Omit<RequestDiscussionState, 'epoch'>>,
) {
  const current = requestDiscussionResource.peek(key)
  if (current?.epoch !== epoch) return false
  requestDiscussionResource.write(key, { ...current, ...patch })
  return true
}

export function updateRequestDiscussionCollection(
  key: string,
  epoch: number,
  transform: (current: DiscussionCollection) => DiscussionCollection,
) {
  const current = requestDiscussionResource.peek(key)
  if (current?.epoch !== epoch) return
  const collection = transform(current.collection)
  if (collection === current.collection) return
  updateRequestDiscussion(key, epoch, { collection })
}

export function stopFollowingRequestDiscussion(key: string, epoch: number) {
  updateRequestDiscussion(key, epoch, { caughtUp: false })
}

export async function syncRequestDiscussionProjections(
  key: string,
  epoch: number,
  requestHead: string,
  load: LoadProjectionPage,
) {
  const retained = requestDiscussionResource.peek(key)
  if (retained?.epoch !== epoch || requestHead === retained.projectedHead) return
  const requested = projectionRequests.get(key)
  if (requested?.epoch === epoch && requested.head === requestHead) return
  const { collection } = retained
  if (retained.projectedHead === null || !collection.order.some((id) => collection.byId.get(id)?.anchor?.lines)) {
    updateRequestDiscussion(key, epoch, { projectedHead: requestHead })
    return
  }
  const request = { epoch, head: requestHead }
  projectionRequests.set(key, request)
  const current = () => projectionRequests.get(key) === request && requestDiscussionResource.peek(key)?.epoch === epoch
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
    } while (cursor && covered < collection.order.length)
    updateRequestDiscussionCollection(key, epoch, (latest) => pages.reduce(
      (next, page) => mergeRefreshedDiscussionPage(next, page),
      latest,
    ))
    updateRequestDiscussion(key, epoch, { projectedHead: requestHead })
  } catch (error) {
    if (current()) {
      updateRequestDiscussion(key, epoch, {
        error: resourceErrorMessage(error, 'Discussion positions could not be refreshed.'),
      })
    }
  } finally {
    if (projectionRequests.get(key) === request) projectionRequests.delete(key)
  }
}

export function readRequestDiscussionScroll(key: string) {
  return requestDiscussionResource.peek(key)?.scrollTop ?? 0
}

export function writeRequestDiscussionScroll(key: string, scrollTop: number) {
  const state = requestDiscussionResource.peek(key)
  if (state) requestDiscussionResource.write(key, { ...state, scrollTop })
}

function retainsDiscussion(state: RequestDiscussionState, discussionId: string | undefined) {
  return !discussionId || state.collection.order.includes(discussionId)
}

export function loadRequestDiscussion({
  key,
  focusedDiscussionId,
  load,
}: {
  key: string
  focusedDiscussionId?: string
  load: () => Promise<RequestDiscussionPage | null>
}): true | Promise<true> {
  const retained = requestDiscussionResource.peek(key)
  if (retained && retainsDiscussion(retained, focusedDiscussionId)) return true
  if (retained) {
    const stillRetained = (): true => {
      if (requestDiscussionResource.peek(key)?.epoch !== retained.epoch) throw new Error('Resource is no longer available.')
      return true
    }
    return load().then((incoming) => {
      stillRetained()
      if (incoming) {
        updateRequestDiscussionCollection(key, retained.epoch, (current) => mergeRefreshedDiscussionPage(current, incoming))
      }
      return stillRetained()
    }, stillRetained)
  }
  return requestDiscussionResource.load(key, '', () => loadRequestDiscussionState(load)).then((state) => retainsDiscussion(state, focusedDiscussionId)
    ? true
    : loadRequestDiscussion({ key, focusedDiscussionId, load }))
}
