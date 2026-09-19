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

type RepliesSession = DiscussionRepliesState & {
  read: () => RepliesSession | null
  target: { promise: Promise<boolean>; replyId: string } | null
  update: (transform: (current: RepliesSession) => DiscussionRepliesState & Partial<Pick<RepliesSession, 'target'>>) => void
}

export const requestDiscussionRepliesResource = createCachedResource<RepliesSession>({
  maxEntries: 500,
  maxWeight: 4 * 1024 * 1024,
  weightOf: ({ replies }) => JSON.stringify(replies).length * 2,
})

export function openRequestDiscussionReplies(key: string): RepliesSession {
  const cached = requestDiscussionRepliesResource.peek(key)
  if (cached) return cached
  const update: RepliesSession['update'] = (transform) => {
    const current = read()
    if (current) requestDiscussionRepliesResource.write(key, { ...current, ...transform(current) })
  }
  const read = () => {
    const current = requestDiscussionRepliesResource.peek(key)
    return current?.update === update ? current : null
  }
  const session = { ...createDiscussionRepliesState(), read, target: null, update }
  requestDiscussionRepliesResource.write(key, session)
  return session
}

export function createRequestDiscussionReplyReads(
  session: RepliesSession,
  loadReplies: (page: Pick<LoadRepliesInput, 'before' | 'reply'>) => Promise<RequestDiscussionRepliesPage>,
  latestReplies: RequestDiscussionReplyView[],
  hasOlderReplies: boolean,
) {
  function loadOlderReplies() {
    const current = session.read()
    if (!current || !hasOlderReplies || current.page.loading) return
    return loadReplyPage(beforePositionForNextReplyPage(current, latestReplies))
  }

  async function loadReplyPage(before: number | undefined) {
    session.update((current) => updateReplyPage(current, { error: null, loading: true }))
    try {
      const page = await loadReplies({ before })
      session.update((current) => mergeReplyPage(current, page, latestReplies, before === undefined))
    } catch (error) {
      session.update((current) => updateReplyPage(current, {
        error: resourceErrorMessage(error, 'Earlier replies could not be loaded.'),
        loading: false,
      }))
    }
  }

  function loadReplyTarget(replyId: string): Promise<boolean> {
    const current = session.read()
    if (!current) return Promise.resolve(false)
    if (mergeDiscussionReplies(current.replies, latestReplies).some((reply) => reply.id === replyId)) {
      return Promise.resolve(true)
    }
    if (current.target) {
      if (current.target.replyId === replyId) return current.target.promise
      return current.target.promise.then(() => loadReplyTarget(replyId))
    }

    session.update((state) => updateReplyPage(state, { error: null, loading: true }))
    const operation = (async () => {
      try {
        const page = await loadReplies({ reply: replyId })
        session.update((state) => mergeReplyTarget(state, page, latestReplies))
        return page.replies.some((reply) => reply.id === replyId)
      } catch (error) {
        session.update((state) => updateReplyPage(state, {
          error: resourceErrorMessage(error, 'Linked reply could not be loaded.'),
          loading: false,
        }))
        return false
      }
    })()
    session.update((state) => ({ ...state, target: { promise: operation, replyId } }))
    void operation.finally(() => {
      session.update((state) => state.target?.promise === operation ? { ...state, target: null } : state)
    })
    return operation
  }

  return { loadOlderReplies, loadReplyTarget }
}
