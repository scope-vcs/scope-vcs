import type { RequestEventResponse } from '@/api/types.generated'
import { actorHandle } from './request-actor'

export type RequestRevisionPush = {
  actor: RequestEventResponse['actor']
  createdAtUnix: number
  /** Also the revision id: a push creates the revision it names. */
  id: string
  newHeadOid: string
  note: string | null
  oldHeadOid: string
  /** The revision number shown for it everywhere. */
  position: number
}

/** Revision pushes from request activity, newest first. */
export function requestRevisionPushes(events: readonly RequestEventResponse[]): RequestRevisionPush[] {
  return events
    .flatMap((event) => {
      if (!('RevisionPushed' in event.payload)) return []
      const payload = event.payload.RevisionPushed
      return [{
        actor: event.actor,
        createdAtUnix: event.created_at_unix,
        id: event.id,
        newHeadOid: payload.new_head_oid,
        note: payload.note,
        oldHeadOid: payload.old_head_oid,
        position: event.position,
      }]
    })
    .sort((left, right) => right.position - left.position)
}

/** Matches every word against the revision number, pusher, note and heads. */
export function searchRequestRevisionPushes(pushes: readonly RequestRevisionPush[], query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return pushes
  return pushes.filter((push) => {
    const text = [
      `revision ${push.position}`,
      actorHandle(push.actor),
      push.note ?? '',
      push.oldHeadOid,
      push.newHeadOid,
    ].join(' ').toLowerCase()
    return words.every((word) => text.includes(word))
  })
}
