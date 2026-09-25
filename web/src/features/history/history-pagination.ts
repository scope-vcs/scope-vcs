import type { HistoryPageResponse } from '@/api/types.generated'

export type LoadedHistory = Pick<HistoryPageResponse, 'entries' | 'next_cursor'>

export function appendHistoryPage(
  current: LoadedHistory,
  page: HistoryPageResponse,
  before: string,
): LoadedHistory {
  if (current.next_cursor !== before) return current
  // Generation-bound cursor pages contain distinct source actions.
  return { entries: [...current.entries, ...page.entries], next_cursor: page.next_cursor }
}
