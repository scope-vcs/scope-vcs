import { createCachedResource } from '../../lib/cached-resource'
import { createDiscussionRepliesState, type DiscussionRepliesState } from './request-discussion-replies-model'

type RepliesSession = DiscussionRepliesState & {
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
    const current = requestDiscussionRepliesResource.peek(key)
    if (current?.update === update) {
      requestDiscussionRepliesResource.write(key, { ...current, ...transform(current) })
    }
  }
  const session = { ...createDiscussionRepliesState(), target: null, update }
  requestDiscussionRepliesResource.write(key, session)
  return session
}
