import type { HistoryEntrySummaryResponse } from '@/api/types.generated'

export const VISIBILITY_TIMELINE_ENTRY_LIMIT = 50

export type VisibilityDirection = 'public' | 'private'

export type VisibilityTimelineBar = {
  direction: VisibilityDirection
  entry: HistoryEntrySummaryResponse
  label: string | null
  signedCount: number
}

export function visibilityTimelineBars(newestFirst: readonly HistoryEntrySummaryResponse[]): VisibilityTimelineBar[] {
  const oldestFirst = newestFirst.slice(0, VISIBILITY_TIMELINE_ENTRY_LIMIT).reverse()
  const bars = oldestFirst.flatMap(entryBars)
  const newest = oldestFirst.at(-1)
  const labeled = new Set([bars.find((bar) => bar.entry === newest), largestBar(bars)])
  return bars.map((bar) => labeled.has(bar) ? { ...bar, label: String(Math.abs(bar.signedCount)) } : bar)
}

function entryBars(entry: HistoryEntrySummaryResponse): VisibilityTimelineBar[] {
  const { made_public_count: madePublic, made_private_count: madePrivate } = entry.visibility_summary
  return [
    ...madePublic > 0 ? [{ direction: 'public' as const, entry, label: null, signedCount: madePublic }] : [],
    ...madePrivate > 0 ? [{ direction: 'private' as const, entry, label: null, signedCount: -madePrivate }] : [],
  ]
}

function largestBar(bars: readonly VisibilityTimelineBar[]) {
  return bars.reduce<VisibilityTimelineBar | undefined>(
    (largest, bar) => !largest || Math.abs(bar.signedCount) > Math.abs(largest.signedCount) ? bar : largest,
    undefined,
  )
}
