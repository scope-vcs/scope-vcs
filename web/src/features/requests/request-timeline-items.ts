import type { RequestEventResponse } from '@/api/types.generated'

export type RequestRevisionPush = {
  actor: RequestEventResponse['actor']
  createdAtUnix: number
  /** Also the revision id: a push creates the revision it names. */
  id: string
  newHeadOid: string
  note: string | null
  oldHeadOid: string
  position: number
}

export type RequestTimelineItem<Discussion> =
  | { discussion: Discussion; kind: 'discussion' }
  | { kind: 'revision'; push: RequestRevisionPush }

export function requestRevisionPushes(events: readonly RequestEventResponse[]): RequestRevisionPush[] {
  return events.flatMap((event) => {
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
}

/**
 * Discussions and events share the request's activity positions, so pushes
 * fall exactly between the discussions around them. While earlier discussions
 * are unloaded, pushes older than the oldest loaded one wait for them.
 */
export function requestTimelineItems<Discussion extends { opened_position: number }>(
  discussions: readonly Discussion[],
  pushes: readonly RequestRevisionPush[],
  hasEarlierDiscussions: boolean,
): RequestTimelineItem<Discussion>[] {
  const oldestLoaded = Math.min(...discussions.map(({ opened_position }) => opened_position))
  const visible = pushes
    .filter(({ position }) => !hasEarlierDiscussions || position > oldestLoaded)
    .sort((left, right) => left.position - right.position)
  const items: RequestTimelineItem<Discussion>[] = []
  let next = 0
  for (const discussion of discussions) {
    while (next < visible.length && visible[next].position < discussion.opened_position) {
      items.push({ kind: 'revision', push: visible[next++] })
    }
    items.push({ discussion, kind: 'discussion' })
  }
  for (const push of visible.slice(next)) items.push({ kind: 'revision', push })
  return items
}
