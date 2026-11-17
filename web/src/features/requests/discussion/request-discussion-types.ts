import type {
  RequestActivityPageResponse,
  RequestDiscussionChangesResponse,
  RequestDiscussionMutationResponse,
  RequestDiscussionPageResponse,
  RequestDiscussionReplyMutationResponse,
  RequestDiscussionReplyResponse,
  RequestDiscussionSummaryResponse,
} from '@/api/types.generated'

export type RequestDiscussionReply = RequestDiscussionReplyResponse
export type RequestDiscussion = RequestDiscussionSummaryResponse
export type RequestDiscussionPage = RequestDiscussionPageResponse
export type RequestDiscussionChanges = RequestDiscussionChangesResponse
export type RequestDiscussionMutation = RequestDiscussionMutationResponse
export type RequestDiscussionReplyMutation = RequestDiscussionReplyMutationResponse
export type RequestActivityPage = RequestActivityPageResponse

type DiscussionPendingState = 'failed' | 'sending'

export type RequestDiscussionView = RequestDiscussion & {
  expanded?: boolean
  initiallyResolved?: boolean
  pending?: DiscussionPendingState
}

export type RequestDiscussionReplyView = RequestDiscussionReply & {
  optimistic_reply_to_reply_id?: string
  optimistic_wait_after_reply?: boolean
  pending?: DiscussionPendingState
}
