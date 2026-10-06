import type { HistoryEntrySummaryResponse } from '@/api/types.generated'

export type VisibilityDirection = 'public' | 'private'

export type VisibilityTimelineBar = {
  direction: VisibilityDirection
  entry: HistoryEntrySummaryResponse
  label: string | null
  signedCount: number
}

export function visibilityTimelineBars(newestFirst: readonly HistoryEntrySummaryResponse[]): VisibilityTimelineBar[] {
  const oldestFirst = [...newestFirst].reverse()
  const labeledIds = new Set([oldestFirst.at(-1)?.id, largestPublicId(oldestFirst)])
  return oldestFirst.flatMap((entry) => {
    const { made_public_count: madePublic, made_private_count: madePrivate } = entry.visibility_summary
    const labeled = labeledIds.has(entry.id)
    const bars: VisibilityTimelineBar[] = []
    if (madePublic > 0) {
      bars.push({ direction: 'public', entry, label: labeled ? String(madePublic) : null, signedCount: madePublic })
    }
    if (madePrivate > 0) {
      bars.push({
        direction: 'private',
        entry,
        label: labeled && madePublic === 0 ? String(madePrivate) : null,
        signedCount: -madePrivate,
      })
    }
    return bars
  })
}

function largestPublicId(oldestFirst: readonly HistoryEntrySummaryResponse[]) {
  return oldestFirst.reduce<HistoryEntrySummaryResponse | undefined>(
    (largest, entry) => !largest || entry.visibility_summary.made_public_count > largest.visibility_summary.made_public_count
      ? entry
      : largest,
    undefined,
  )?.id
}
