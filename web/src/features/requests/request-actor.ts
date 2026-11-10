import type { RequestActorSummaryResponse } from '@/api/types.generated'

export type RecordedActor = RequestActorSummaryResponse | null

export const DELETED_USER_LABEL = 'Deleted user'

export function actorHandle(actor: RecordedActor) {
  return actor?.handle ?? DELETED_USER_LABEL
}
