import type {
  RequestAttentionMutationResponse,
  RequestQueueItemResponse,
  RequestQueueSection,
} from '../../api/types.generated'
import { REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages } from './request-list-model'

/** The attention commands the browser applies before the server answers. */
export type RequestInstantCommand =
  { action: 'restore' | 'settle' } | { action: 'snooze'; until_unix: number }

/**
 * A row the viewer moved that the loaded queue does not show yet. The queue
 * cache stays server truth; moves sit on top of it until a refresh lands.
 */
export type RequestAttentionMove = {
  item: RequestQueueItemResponse
  command: RequestInstantCommand
  atUnix: number
  /** The server's answer, once it has one. */
  confirmed: RequestAttentionMutationResponse | null
}

export function isInstantCommand(command: { action: string }): command is RequestInstantCommand {
  return command.action === 'settle' || command.action === 'snooze' || command.action === 'restore'
}

/** Where a moved row shows while the queue catches up. */
export function movedRow(move: RequestAttentionMove): {
  item: RequestQueueItemResponse
  section: RequestQueueSection
} {
  const { item, command, confirmed } = move
  const section = command.action === 'restore' ? 'active' : 'set_aside'
  // The API orders a row by the later of its last update and its attention
  // change, so a move never pulls the row's time backwards.
  const atUnix = Math.max(item.attention_at_unix, move.atUnix)
  if (confirmed) {
    return { section, item: { ...item, ...confirmed, attention_at_unix: atUnix } }
  }
  const attention =
    command.action === 'restore'
      ? { state: 'active', reason: 'restored', snoozed_until_unix: null, can_set_aside: true, can_restore: false } as const
      : command.action === 'snooze'
        ? { state: 'snoozed', reason: 'snoozed', snoozed_until_unix: command.until_unix, can_set_aside: false, can_restore: true } as const
        : { state: 'settled', reason: 'settled', snoozed_until_unix: null, can_set_aside: false, can_restore: true } as const
  return {
    section,
    item: { ...item, attention: { ...item.attention, ...attention }, attention_at_unix: atUnix },
  }
}

/** The order the API serves a section in: newest attention first, then id. */
function byQueueOrder(a: RequestQueueItemResponse, b: RequestQueueItemResponse) {
  return b.attention_at_unix - a.attention_at_unix || (a.request.id < b.request.id ? -1 : 1)
}

/**
 * Whether the loaded queue already reflects a move. Every write to the viewer's
 * attention record advances its revision, so the row the queue serves either
 * predates the move's answer or includes it. A row the queue no longer loads
 * has nowhere stale to show.
 */
export function queueReflectsMove(pages: RequestQueuePages, move: RequestAttentionMove) {
  if (!move.confirmed) return false
  const loaded = REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => pages[section].requests).find(
    (item) => item.request.id === move.item.request.id,
  )
  return !loaded || loaded.attention.revision >= move.confirmed.attention.revision
}

/** The queue as the viewer should see it: loaded pages with their moves applied. */
export function applyAttentionMoves(
  pages: RequestQueuePages,
  allMoves: readonly RequestAttentionMove[],
): RequestQueuePages {
  const moves = allMoves.filter((move) => !queueReflectsMove(pages, move))
  if (!moves.length) return pages
  const moved = new Map(moves.map((move) => [move.item.request.id, movedRow(move)]))
  return Object.fromEntries(
    REQUEST_QUEUE_SECTION_ORDER.map((section) => {
      const arrivals = [...moved.values()]
        .filter((row) => row.section === section)
        .map((row) => row.item)
      if (!arrivals.length && !pages[section].requests.some((item) => moved.has(item.request.id))) {
        return [section, pages[section]]
      }
      const staying = pages[section].requests.filter((item) => !moved.has(item.request.id))
      return [section, { ...pages[section], requests: [...arrivals, ...staying].sort(byQueueOrder) }]
    }),
  ) as RequestQueuePages
}
