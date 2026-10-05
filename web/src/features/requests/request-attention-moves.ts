import type {
  RequestAttentionMutationResponse,
  RequestQueueItemResponse,
  RequestQueueSection,
} from '../../api/types.generated'
import { REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages } from './request-list-model'

export type RequestInstantCommand =
  { action: 'restore' | 'settle' } | { action: 'snooze'; until_unix: number }

export type RequestAttentionMove = {
  item: RequestQueueItemResponse
  command: RequestInstantCommand
  atUnix: number
  confirmed: RequestAttentionMutationResponse | null
}

export function isInstantCommand(command: { action: string }): command is RequestInstantCommand {
  return command.action === 'settle' || command.action === 'snooze' || command.action === 'restore'
}

export function movedRow(move: RequestAttentionMove): {
  item: RequestQueueItemResponse
  section: RequestQueueSection
} {
  const { item, command, confirmed } = move
  const section = command.action === 'restore' ? 'active' : 'set_aside'
  const atUnix = Math.max(item.attention_at_unix, move.atUnix)
  if (confirmed) {
    return { section, item: { ...item, ...confirmed, attention_at_unix: atUnix } }
  }
  const attention =
    command.action === 'restore'
      ? { group: 'needs_you', state: 'active', reason: 'restored', snoozed_until_unix: null, can_set_aside: true, can_restore: false } as const
      : command.action === 'snooze'
        ? { group: 'set_aside', state: 'snoozed', reason: 'snoozed', snoozed_until_unix: command.until_unix, can_set_aside: false, can_restore: true } as const
        : { group: 'set_aside', state: 'settled', reason: 'settled', snoozed_until_unix: null, can_set_aside: false, can_restore: true } as const
  return {
    section,
    item: { ...item, attention: { ...item.attention, ...attention }, attention_at_unix: atUnix },
  }
}

function byQueueOrder(a: RequestQueueItemResponse, b: RequestQueueItemResponse) {
  return b.attention_at_unix - a.attention_at_unix || (a.request.id < b.request.id ? -1 : 1)
}

export function queueReflectsMove(pages: RequestQueuePages, move: RequestAttentionMove) {
  if (!move.confirmed) return false
  const loaded = REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => pages[section].requests).find(
    (item) => item.request.id === move.item.request.id,
  )
  return !loaded || loaded.attention.revision >= move.confirmed.attention.revision
}

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
