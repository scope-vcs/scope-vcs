import { runRequestContentSubmission } from '../attachments/request-attachment-drafts'
import type { RequestParams } from '@/api/types'
import type { RepoChangeEvent, RequestActorSummaryResponse } from '@/api/types.generated'
import { useRepoChangeSubscription } from '@/features/repo-detail/repo-layout-context'
import { resourceErrorMessage, useCachedResource } from '../../../lib/use-cached-resource'
import {
  useCallback,
  useEffect,
  useRef,
} from 'react'
import {
  loadRequestDiscussionState,
  requestDiscussionResource,
  stopFollowingRequestDiscussion,
  syncRequestDiscussionProjections,
  updateRequestDiscussion,
  updateRequestDiscussionCollection,
  type RequestDiscussionState,
} from './request-discussion-cache'
import { createRequestDiscussionSync } from './request-discussion-sync'
import {
  insertOptimisticDiscussion,
  markDiscussionFailed,
  markDiscussionRead,
  mergeDiscussion,
  orderedDiscussions,
  reconcileDiscussionMutation,
  type DiscussionCollection,
} from './request-discussion-model'
import type {
  CreateDiscussionInput,
  LoadDiscussionsInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
} from './request-discussion-api'
import type {
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionView,
} from './request-discussion-types'

type RequestDiscussionSync = ReturnType<typeof createRequestDiscussionSync>

export type RequestDiscussionActions = {
  create: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  load: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  loadFirstPage: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage | null>
  loadChanges: (
    input: RequestParams & { after: number },
  ) => Promise<RequestDiscussionChanges>
  markRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  resolve: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
}

export function useRequestDiscussionState(key: string, loadFirstPage: () => Promise<RequestDiscussionPage | null>) {
  const load = useCallback(() => loadRequestDiscussionState(loadFirstPage), [loadFirstPage])
  return useCachedResource({
    fallbackError: 'Discussion is unavailable.',
    identity: key,
    load,
    resource: requestDiscussionResource,
  })
}

export function useRequestDiscussionStore({
  actions,
  actor,
  cacheKey: key,
  params,
  requestHead,
  state,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummaryResponse
  cacheKey: string
  params: RequestParams
  requestHead: string
  state: RequestDiscussionState
}) {
  const { collection, epoch, error, loadingMore } = state
  const read = useCallback(() => {
    const current = requestDiscussionResource.peek(key)
    return current?.epoch === epoch ? current : null
  }, [epoch, key])
  const updateCollection = useCallback(
    (transform: (current: DiscussionCollection) => DiscussionCollection) =>
      updateRequestDiscussionCollection(key, epoch, transform),
    [epoch, key],
  )
  const setError = useCallback((next: string | null) => { updateRequestDiscussion(key, epoch, { error: next }) }, [epoch, key])
  const setLoadingMore = useCallback((next: boolean) => { updateRequestDiscussion(key, epoch, { loadingMore: next }) }, [epoch, key])

  const syncRef = useRef<RequestDiscussionSync | null>(null)
  const loadChangesRef = useRef((after: number) => actions.loadChanges({ ...params, after }))
  useEffect(() => {
    loadChangesRef.current = (after) => actions.loadChanges({ ...params, after })
  }, [actions, params])
  const catchUp = useCallback((input?: { lagged?: boolean; target?: number }) => {
    void syncRef.current?.catchUp(input)
  }, [])
  useEffect(() => {
    const retained = requestDiscussionResource.peek(key)
    if (retained?.epoch !== epoch) return
    const empty = { ...retained.collection, byId: new Map(), nextCursor: null, order: [] }
    const sync = createRequestDiscussionSync({
      getCollection: () => read()?.collection ?? empty,
      loadChanges: (after) => {
        if (read()) return loadChangesRef.current(after)
        sync.stop()
        return Promise.resolve({ discussions: [], through_position: after, has_more: false })
      },
      onCatchUpError: (failure) => setError(resourceErrorMessage(failure, 'New discussion activity could not be loaded.')),
      setCollection: (next) => updateCollection(() => next),
    })
    sync.reset(key)
    syncRef.current = sync
    if (!retained.caughtUp) void sync.catchUp()
    return () => {
      sync.stop()
      syncRef.current = null
      setLoadingMore(false)
      stopFollowingRequestDiscussion(key, epoch)
    }
  }, [epoch, key, read, setError, setLoadingMore, updateCollection])
  useEffect(() => {
    void syncRequestDiscussionProjections(key, epoch, requestHead, (page) => actions.load({ ...params, ...page }))
  }, [actions, epoch, key, params, requestHead])

  const onRepoChange = useCallback(
    (event: RepoChangeEvent) => {
      if (event.kind === 'Lagged') {
        catchUp({ lagged: true })
        return
      }
      if (
        typeof event.kind === 'object' &&
        'RequestTimelineChanged' in event.kind &&
        event.kind.RequestTimelineChanged.request_id === params.request_id &&
        event.kind.RequestTimelineChanged.through_position >
          (read()?.collection.snapshotVersion ?? 0)
      ) {
        catchUp({ target: event.kind.RequestTimelineChanged.through_position })
      }
    },
    [catchUp, params.request_id, read],
  )
  useRepoChangeSubscription(onRepoChange)

  const loadMore = useCallback(async () => {
    const current = read()
    const sync = syncRef.current
    const cursor = current?.collection.nextCursor
    if (!current || !sync || !cursor || current.loadingMore) return
    setLoadingMore(true)
    setError(null)
    try {
      await sync.paginate(cursor, () => actions.load({ ...params, cursor }))
    } catch (requestError) {
      setError(resourceErrorMessage(requestError, 'Older discussions could not be loaded.'))
    } finally {
      if (syncRef.current === sync) setLoadingMore(false)
    }
  }, [
    actions,
    read,
    params,
    setError,
    setLoadingMore,
  ])

  const create = useCallback(
    async (
      body: string,
      clientDiscussionId: string = crypto.randomUUID(),
    ) => runRequestContentSubmission(clientDiscussionId, async () => {
      const optimistic = optimisticDiscussion({
        actor,
        body,
        clientDiscussionId,
        requestId: params.request_id,
      })
      updateCollection((current) =>
        insertOptimisticDiscussion(current, optimistic),
      )
      setError(null)
      try {
        const result = await actions.create({
          ...params,
          anchor: null,
          body_markdown: body,
          client_discussion_id: clientDiscussionId,
        })
        updateCollection((current) =>
          reconcileDiscussionMutation(
            current,
            result.discussion,
            clientDiscussionId,
          ),
        )
        catchUp({ target: result.discussion.last_activity_position })
        return true
      } catch (requestError) {
        updateCollection((current) => markDiscussionFailed(current, clientDiscussionId))
        setError(resourceErrorMessage(requestError, 'Discussion could not be posted.'))
        return false
      }
    }),
    [actions, actor, catchUp, params, setError, updateCollection],
  )

  const retry = useCallback(
    (discussion: RequestDiscussionView) =>
      discussion.body_markdown
        ? create(discussion.body_markdown, discussion.id)
        : Promise.resolve(false),
    [create],
  )

  const patch = useCallback(
    (discussion: RequestDiscussion) => {
      if (discussion.request_id !== params.request_id) {
        return
      }
      updateCollection((current) => mergeDiscussion(current, discussion))
      catchUp({ target: discussion.last_activity_position })
    },
    [catchUp, params.request_id, updateCollection],
  )

  const markRead = useCallback(
    async (discussion: RequestDiscussion) => {
      if (discussion.unread_count === 0) return
      updateCollection((current) =>
        markDiscussionRead(current, discussion.id),
      )
      try {
        await actions.markRead({
          ...params,
          discussion_id: discussion.id,
          through_position: discussion.last_activity_position,
        })
      } catch {
        updateCollection((current) => {
          const existing = current.byId.get(discussion.id)
          if (
            !existing ||
            existing.last_activity_position !==
              discussion.last_activity_position
          ) {
            return current
          }
          return mergeDiscussion(current, {
            ...existing,
            unread_count: discussion.unread_count,
          })
        })
      }
    },
    [actions, params, updateCollection],
  )

  const resolve = useCallback(
    async (discussion: RequestDiscussion) => {
      setError(null)
      try {
        const result = await actions.resolve({
          ...params,
          discussion_id: discussion.id,
        })
        patch(result.discussion)
      } catch (requestError) {
        setError(resourceErrorMessage(requestError, 'Discussion could not be resolved.'))
      }
    },
    [actions, params, patch, setError],
  )

  const setExpanded = useCallback(
    (discussionId: string, expanded: boolean) => {
      updateCollection((current) => {
        const discussion = current.byId.get(discussionId)
        return discussion
          ? mergeDiscussion(current, { ...discussion, expanded })
          : current
      })
    },
    [updateCollection],
  )

  return {
    cacheKey: key,
    collection,
    create,
    discussions: orderedDiscussions(collection),
    error,
    loadMore,
    loadingMore,
    markRead,
    patch,
    retry,
    setExpanded,
    resolve,
  }
}

function optimisticDiscussion({
  actor,
  body,
  clientDiscussionId,
  requestId,
}: {
  actor: RequestActorSummaryResponse
  body: string
  clientDiscussionId: string
  requestId: string
}): RequestDiscussionView {
  const position = Number.MAX_SAFE_INTEGER
  return {
    author: { kind: 'user', id: actor.id, handle: actor.handle },
    anchor: null,
    body_markdown: body,
    client_discussion_id: clientDiscussionId,
    created_at_unix: Math.floor(Date.now() / 1000),
    id: clientDiscussionId,
    last_activity_position: position,
    latest_replies: [],
    opened_position: position,
    pending: 'sending',
    read_through_position: position,
    reply_count: 0,
    request_id: requestId,
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}

