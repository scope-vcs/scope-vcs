import type {
  RequestDiscussionAuthorResponse,
  RequestDiscussionResolverResponse,
} from '@/api/types.generated'
import { DELETED_USER_LABEL } from './request-actor'

export type DiscussionAuthor = RequestDiscussionAuthorResponse | null

export function discussionAuthorName(author: DiscussionAuthor) {
  if (author === null) return DELETED_USER_LABEL
  return author.kind === 'user' ? author.handle : author.account.login
}

export function discussionAuthorAvatarUrl(author: DiscussionAuthor) {
  return author?.kind === 'github' ? author.account.avatar_url : null
}

export function discussionAuthorSourceUrl(author: DiscussionAuthor) {
  return author?.kind === 'github' ? author.source_url : null
}

export function discussionAuthorSourceLabel(author: DiscussionAuthor) {
  if (author?.kind !== 'github') return null
  const comment = /#discussion_r(\d+)$/.exec(author.source_url)
  const review = /#pullrequestreview-(\d+)$/.exec(author.source_url)
  const target = comment
    ? `comment ${comment[1]}`
    : review
      ? `review ${review[1]}`
      : 'comment'
  return `via GitHub: ${author.account.login}'s ${target}`
}

export function discussionAuthorUserId(author: DiscussionAuthor) {
  return author?.kind === 'user' ? author.id : null
}

export function isSameDiscussionAuthor(left: DiscussionAuthor, right: DiscussionAuthor) {
  if (left === null || right === null) return false
  if (left.kind === 'user' && right.kind === 'user') return left.id === right.id
  if (left.kind === 'github' && right.kind === 'github') {
    return left.account.github_user_id === right.account.github_user_id
  }
  return false
}

export function discussionResolverName(resolver: RequestDiscussionResolverResponse) {
  return resolver.kind === 'user' ? resolver.handle : resolver.account.login
}
