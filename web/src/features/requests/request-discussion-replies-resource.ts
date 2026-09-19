import { createCachedResource } from '../../lib/cached-resource'
import { resourceErrorMessage } from '../../lib/use-cached-resource'
import type { LoadRepliesInput, RequestDiscussionRepliesPage } from './request-discussion-api'
import {
  beforePositionForNextReplyPage,
  createDiscussionRepliesState,
  mergeDiscussionReplies,
  mergeReplyPage,
  mergeReplyTarget,
  updateReplyPage,
  type DiscussionRepliesState,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'

type ReplyRequestParams = Omit<
  LoadRepliesInput,
  'before' | 'discussion_id' | 'reply'
>

export type RequestDiscussionRepliesReadContext = {
  discussionId: string
  latestReplies: RequestDiscussionReplyView[]
  loadReplies: (
    input: LoadRepliesInput,
  ) => Promise<RequestDiscussionRepliesPage>
  params: ReplyRequestParams
}

export type RequestDiscussionRepliesSession = DiscussionRepliesState & {
  read: () => RequestDiscussionRepliesSession | null
  target: { promise: Promise<boolean>; replyId: string } | null
  update: (
    transform: (
      current: RequestDiscussionRepliesSession,
    ) => DiscussionRepliesState &
      Partial<Pick<RequestDiscussionRepliesSession, 'target'>>,
  ) => void
}

export const requestDiscussionRepliesResource = createCachedResource<RequestDiscussionRepliesSession>({
  maxEntries: 500,
  maxWeight: 4 * 1024 * 1024,
  weightOf: ({ replies }) => JSON.stringify(replies).length * 2,
})

export function openRequestDiscussionReplies(
  key: string,
): RequestDiscussionRepliesSession {
  const cached = requestDiscussionRepliesResource.peek(key)
  if (cached) return cached
  const update: RequestDiscussionRepliesSession['update'] = (transform) => {
    const current = requestDiscussionRepliesResource.peek(key)
    if (current?.update === update) {
      requestDiscussionRepliesResource.write(key, { ...current, ...transform(current) })
    }
  }
  const read = () => {
    const current = requestDiscussionRepliesResource.peek(key)
    return current?.update === update ? current : null
  }
  const session = {
    ...createDiscussionRepliesState(),
    read,
    target: null,
    update,
  }
  requestDiscussionRepliesResource.write(key, session)
  return session
}

export function loadOlderRequestDiscussionReplies(
  session: RequestDiscussionRepliesSession,
  context: RequestDiscussionRepliesReadContext,
  hasOlderReplies: boolean,
) {
  const current = session.read()
  if (!current || !hasOlderReplies || current.page.loading) return
  return loadReplyPage(
    session,
    context,
    beforePositionForNextReplyPage(current, context.latestReplies),
  )
}

export function loadLinkedRequestDiscussionReply(
  session: RequestDiscussionRepliesSession,
  context: RequestDiscussionRepliesReadContext,
  replyId: string,
): Promise<boolean> {
  const current = session.read()
  if (!current) return Promise.resolve(false)
  if (
    mergeDiscussionReplies(current.replies, context.latestReplies).some(
      (reply) => reply.id === replyId,
    )
  ) {
    return Promise.resolve(true)
  }
  if (current.target) {
    if (current.target.replyId === replyId) return current.target.promise
    return current.target.promise.then(() =>
      loadLinkedRequestDiscussionReply(session, context, replyId),
    )
  }

  session.update((state) =>
    updateReplyPage(state, { error: null, loading: true }),
  )
  const operation = loadReplyTarget(session, context, replyId)
  session.update((state) => ({
    ...state,
    target: { promise: operation, replyId },
  }))
  void operation.finally(() => {
    session.update((state) =>
      state.target?.promise === operation
        ? { ...state, target: null }
        : state,
    )
  })
  return operation
}

async function loadReplyPage(
  session: RequestDiscussionRepliesSession,
  context: RequestDiscussionRepliesReadContext,
  before: number | undefined,
) {
  session.update((state) =>
    updateReplyPage(state, { error: null, loading: true }),
  )
  try {
    const page = await context.loadReplies({
      ...context.params,
      before,
      discussion_id: context.discussionId,
    })
    session.update((state) =>
      mergeReplyPage(
        state,
        page,
        context.latestReplies,
        before === undefined,
      ),
    )
  } catch (error) {
    session.update((state) =>
      updateReplyPage(state, {
        error: resourceErrorMessage(
          error,
          'Earlier replies could not be loaded.',
        ),
        loading: false,
      }),
    )
  }
}

async function loadReplyTarget(
  session: RequestDiscussionRepliesSession,
  context: RequestDiscussionRepliesReadContext,
  replyId: string,
) {
  try {
    const page = await context.loadReplies({
      ...context.params,
      discussion_id: context.discussionId,
      reply: replyId,
    })
    session.update((state) =>
      mergeReplyTarget(state, page, context.latestReplies),
    )
    return page.replies.some((reply) => reply.id === replyId)
  } catch (error) {
    session.update((state) =>
      updateReplyPage(state, {
        error: resourceErrorMessage(
          error,
          'Linked reply could not be loaded.',
        ),
        loading: false,
      }),
    )
    return false
  }
}
