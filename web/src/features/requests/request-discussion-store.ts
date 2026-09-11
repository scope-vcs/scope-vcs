import { runRequestContentSubmission } from './request-attachment-drafts'
import type { RequestParams } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useRepoChangeSubscription } from '@/features/repo-detail/repo-layout-context'
import {
  useCallback,
  useEffect,
  useMemo,
  useSyncExternalStore,
} from 'react'
import {
  openRequestDiscussion,
  requestDiscussionCacheKey,
  requestDiscussionResource,
} from './request-discussion-cache'
import {
  insertOptimisticDiscussion,
  markDiscussionFailed,
  markDiscussionRead,
  mergeDiscussion,
  orderedDiscussions,
  reconcileDiscussionMutation,
} from './request-discussion-model'
import type {
  CreateDiscussionInput,
  LoadDiscussionsInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
} from './request-discussion-api'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionView,
} from './request-discussion-types'

export type RequestDiscussionActions = {
  create: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  load: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  loadChanges: (
    input: RequestParams & { after: number },
  ) => Promise<RequestDiscussionChanges>
  markRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  resolve: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
}

export function useRequestDiscussionStore({
  actions,
  actor,
  initialPage,
  params,
  repoId,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  initialPage: RequestDiscussionPage
  params: RequestParams
  repoId: string
}) {
  const key = requestDiscussionCacheKey({
    repoId,
    requestId: params.request_id,
    viewerId: actor.id,
  })
  const session = useMemo(
    () => openRequestDiscussion(key, initialPage, (after) => actions.loadChanges({ ...params, after })),
    [actions, initialPage, key, params],
  )
  const subscribe = useCallback((listener: () => void) => requestDiscussionResource.subscribe(key, listener), [key])
  const read = useCallback(() => requestDiscussionResource.peek(key) ?? session, [key, session])
  const { collection, error, loadingMore } = useSyncExternalStore(subscribe, read, () => session)
  const { sync, updateCollection, setError, setLoadingMore } = session

  useEffect(() => { void session.refresh(initialPage) }, [initialPage, session])

  const onRepoChange = useCallback(
    (event: RepoChangeEvent) => {
      if (event.kind === 'Lagged') {
        void sync.catchUp({ lagged: true })
        return
      }
      if (
        typeof event.kind === 'object' &&
        'RequestTimelineChanged' in event.kind &&
        event.kind.RequestTimelineChanged.request_id === params.request_id &&
        event.kind.RequestTimelineChanged.through_position >
          read().collection.snapshotVersion
      ) {
        void sync.catchUp({
          target: event.kind.RequestTimelineChanged.through_position,
        })
      }
    },
    [params.request_id, read, sync],
  )
  useRepoChangeSubscription(onRepoChange)

  const loadMore = useCallback(async () => {
    const { collection: current, loadingMore: pending } = read()
    const cursor = current.nextCursor
    if (!cursor || pending) return
    setLoadingMore(true)
    setError(null)
    try {
      await sync.paginate(cursor, () => actions.load({ ...params, cursor }))
    } catch (requestError) {
      setError(messageFor(requestError, 'Older discussions could not be loaded.'))
    } finally {
      setLoadingMore(false)
    }
  }, [
    actions,
    read,
    params,
    setError,
    setLoadingMore,
    sync,
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
        void sync.catchUp({
          target: result.discussion.last_activity_position,
        })
        return true
      } catch (requestError) {
        updateCollection((current) => markDiscussionFailed(current, clientDiscussionId))
        setError(messageFor(requestError, 'Discussion could not be posted.'))
        return false
      }
    }),
    [actions, actor, params, setError, sync, updateCollection],
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
      void sync.catchUp({ target: discussion.last_activity_position })
    },
    [params.request_id, sync, updateCollection],
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
        setError(messageFor(requestError, 'Discussion could not be resolved.'))
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
      }, false)
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
  actor: RequestActorSummary
  body: string
  clientDiscussionId: string
  requestId: string
}): RequestDiscussionView {
  const position = Number.MAX_SAFE_INTEGER
  return {
    author: actor,
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

function messageFor(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback
}
