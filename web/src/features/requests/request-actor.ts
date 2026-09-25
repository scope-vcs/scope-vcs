import type { RequestActorSummaryResponse } from '@/api/types.generated'

/** Work outlives accounts: its author or actor is null once that account is deleted. */
export type RecordedActor = RequestActorSummaryResponse | null

export const DELETED_USER_LABEL = 'Deleted user'

export function actorHandle(actor: RecordedActor) {
  return actor?.handle ?? DELETED_USER_LABEL
}

/** Deleted accounts are nobody in particular, so they match no one. */
export function isSameActor(left: RecordedActor, right: RecordedActor) {
  return left !== null && right !== null && left.id === right.id
}
