import { resourceErrorMessage } from '../../lib/use-cached-resource'
import { useCallback, useMemo, useState, useSyncExternalStore } from 'react'
import { useAuth } from '@clerk/tanstack-react-start'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { requestQueueResource } from './request-queue-cache'
import { openRequestDiscussionReplies, requestDiscussionRepliesResource } from './request-discussion-replies-resource'
import type {
  CreateReplyInput,
  LoadRepliesInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import {
  acknowledgeReply,
  beforePositionForNextReplyPage,
  hasLoadedAllUnreadContent,
  insertOptimisticReply,
  markReplyFailed,
  mergeDiscussionReplies,
  mergeReplyPage,
  mergeReplyTarget,
  updateReplyPage,
} from './request-discussion-replies-model'
import type {
  RequestDiscussion,
  RequestDiscussionReplyMutation,
  RequestDiscussionReplyView,
  RequestDiscussionView,
} from './request-discussion-types'

export type RequestDiscussionThreadActions = {
  createReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  loadReplies: (input: LoadRepliesInput) => Promise<RequestDiscussionRepliesPage>
  reopenAndReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
}

export function useRequestDiscussionReplies({
  actions,
  actor,
  canReply,
  canResolve,
  discussion,
  onExpandedChange,
  onPatch,
  params,
}: {
  actions: RequestDiscussionThreadActions
  actor: { handle: string; id: string }
  canReply: boolean
  canResolve: boolean
  discussion: RequestDiscussionView
  onExpandedChange: (discussionId: string, expanded: boolean) => void
  onPatch: (discussion: RequestDiscussion) => void
  params: { owner: string; repo: string; request_id: string }
}) {
  const { userId } = useAuth()
  const { repo } = useRepoLayout()
  const key = `${repoResourceScope(repo, actor.id)}\0${params.request_id}\0${discussion.id}`
  const session = useMemo(() => openRequestDiscussionReplies(key), [key])
  const subscribe = useCallback((listener: () => void) => requestDiscussionRepliesResource.subscribe(key, listener), [key])
  const read = useCallback(() => requestDiscussionRepliesResource.peek(key) ?? session, [key, session])
  const replyState = useSyncExternalStore(subscribe, read, () => session)
  const setReplyState = session.update
  const [quoteId, setQuoteId] = useState<string | null>(null)

  const availableReplies = mergeDiscussionReplies(
    replyState.replies,
    discussion.latest_replies,
  )
  const loadingReplies = replyState.page.loading
  const replyError = replyState.page.error
  const loadedReplyCount = availableReplies.filter(
    (reply) => !reply.pending,
  ).length
  const olderReplyCount = Math.max(
    discussion.reply_count - loadedReplyCount,
    0,
  )
  const hasOlderReplies = olderReplyCount > 0

  async function loadReplyPage(before: number | undefined) {
    setReplyState((current) =>
      updateReplyPage(current, {
        error: null,
        loading: true,
      }),
    )
    try {
      const page = await actions.loadReplies({
        ...params,
        before,
        discussion_id: discussion.id,
      })
      setReplyState((current) =>
        mergeReplyPage(
          current,
          page,
          discussion.latest_replies,
          before === undefined,
        ),
      )
    } catch (error) {
      setReplyState((current) =>
        updateReplyPage(current, {
          error: resourceErrorMessage(error, 'Earlier replies could not be loaded.'),
          loading: false,
        }),
      )
    }
  }

  function loadOlderReplies() {
    if (!hasOlderReplies || read().page.loading) return
    return loadReplyPage(
      beforePositionForNextReplyPage(
        read(),
        discussion.latest_replies,
      ),
    )
  }

  function loadReplyTarget(replyId: string): Promise<boolean> {
    if (mergeDiscussionReplies(read().replies, discussion.latest_replies).some((reply) => reply.id === replyId)) {
      return Promise.resolve(true)
    }
    const target = read().target
    if (target) {
      if (target.replyId === replyId) return target.promise
      return target.promise.then(() => loadReplyTarget(replyId))
    }

    setReplyState((current) =>
      updateReplyPage(current, { error: null, loading: true }),
    )
    const operation = (async () => {
      try {
        const page = await actions.loadReplies({
          ...params,
          discussion_id: discussion.id,
          reply: replyId,
        })
        setReplyState((current) =>
          mergeReplyTarget(
            current,
            page,
            discussion.latest_replies,
          ),
        )
        return page.replies.some((reply) => reply.id === replyId)
      } catch (error) {
        setReplyState((current) =>
          updateReplyPage(current, {
            error: resourceErrorMessage(error, 'Linked reply could not be loaded.'),
            loading: false,
          }),
        )
        return false
      }
    })()
    setReplyState((current) => ({ ...current, target: { promise: operation, replyId } }))
    void operation.finally(() => {
      setReplyState((current) => current.target?.promise === operation
        ? { ...current, target: null } : current)
    })
    return operation
  }

  async function postReply(
    body: string,
    options: {
      clientReplyId?: string
      replyToReplyId?: string | null
      retryReference?: RequestDiscussionReplyView['reply_to']
      waitAfterReply?: boolean
    } = {},
  ) {
    const clientReplyId = options.clientReplyId ?? crypto.randomUUID()
    const replyToReplyId = options.replyToReplyId === undefined
      ? quoteId
      : options.replyToReplyId
    const waitAfterReply = options.waitAfterReply ?? false
    const replyTarget = options.retryReference ?? (
      replyToReplyId
        ? availableReplies.find((reply) => reply.id === replyToReplyId) ?? null
        : null
    )
    const optimistic = optimisticReply({
      actor,
      body,
      clientReplyId,
      discussion,
      replyTarget,
      replyToReplyId,
      waitAfterReply,
    })
    setReplyState((current) =>
      insertOptimisticReply(
        current,
        optimistic,
        discussion.latest_replies,
      ),
    )
    const input = {
      ...params,
      body_markdown: body,
      client_reply_id: clientReplyId,
      discussion_id: discussion.id,
      reply_to_reply_id: replyToReplyId,
      wait_after_reply: waitAfterReply,
    }
    try {
      const result = await (
        discussion.status === 'Resolved'
          ? actions.reopenAndReply(input)
          : actions.createReply(input)
      )
      setReplyState((current) =>
        acknowledgeReply(current, clientReplyId, result.reply),
      )
      onPatch(result.discussion)
      onExpandedChange(discussion.id, true)
      setQuoteId(null)
      if (waitAfterReply) {
        requestQueueResource.invalidate(repoResourceScope(repo, userId ?? null))
      }
      return true
    } catch (error) {
      setReplyState((current) =>
        updateReplyPage(markReplyFailed(current, clientReplyId), {
          error: resourceErrorMessage(error, 'Reply could not be posted.'),
        }),
      )
      return false
    }
  }

  const canPostReply =
    canReply && (discussion.status !== 'Resolved' || canResolve)
  const rootUnread =
    discussion.opened_position > discussion.read_through_position

  return {
    availableReplies,
    canPostReply,
    hasOlderReplies,
    loadOlderReplies,
    loadReplyTarget,
    loadingReplies,
    olderReplyCount,
    postReply,
    quotedReply: quoteId
      ? availableReplies.find((reply) => reply.id === quoteId) ?? null
      : null,
    replyError,
    setQuoteId,
    unreadContentFullyExposed:
      hasLoadedAllUnreadContent(
        availableReplies,
        discussion.read_through_position,
        discussion.unread_count,
        rootUnread,
      ),
  }
}

function optimisticReply({
  actor,
  body,
  clientReplyId,
  discussion,
  replyTarget,
  replyToReplyId,
  waitAfterReply,
}: {
  actor: { handle: string; id: string }
  body: string
  clientReplyId: string
  discussion: RequestDiscussion
  replyTarget: RequestDiscussionReplyView | RequestDiscussionReplyView['reply_to']
  replyToReplyId: string | null
  waitAfterReply: boolean
}): RequestDiscussionReplyView {
  return {
    author: actor,
    body_markdown: body,
    created_at_unix: Math.floor(Date.now() / 1000),
    discussion_id: discussion.id,
    id: clientReplyId,
    optimistic_reply_to_reply_id: replyTarget ? undefined : replyToReplyId ?? undefined,
    optimistic_wait_after_reply: waitAfterReply || undefined,
    pending: 'sending',
    position: Number.MAX_SAFE_INTEGER,
    reply_to: replyTarget
      ? {
          author: replyTarget.author,
          body_markdown: replyTarget.body_markdown,
          id: replyTarget.id,
          position: replyTarget.position,
        }
      : null,
  }
}

