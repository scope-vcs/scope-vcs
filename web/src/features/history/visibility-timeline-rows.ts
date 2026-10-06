import type { HistoryEntrySummaryResponse } from '@/api/types.generated'

export const VISIBILITY_TIMELINE_ENTRY_LIMIT = 50

export type VisibilityDirection = 'entered' | 'left'

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
  const { entered_count: entered, left_count: left } = entry.visibility_summary
  return [
    ...entered > 0 ? [{ direction: 'entered' as const, entry, label: null, signedCount: entered }] : [],
    ...left > 0 ? [{ direction: 'left' as const, entry, label: null, signedCount: -left }] : [],
  ]
}

function largestBar(bars: readonly VisibilityTimelineBar[]) {
  return bars.reduce<VisibilityTimelineBar | undefined>(
    (largest, bar) => !largest || Math.abs(bar.signedCount) > Math.abs(largest.signedCount) ? bar : largest,
    undefined,
  )
}
