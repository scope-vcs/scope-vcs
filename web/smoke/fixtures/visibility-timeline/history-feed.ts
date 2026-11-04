import type { HistoryEntrySummaryResponse } from '@/api/types.generated'

const DAY = 86_400
const START = Date.UTC(2026, 7, 18) / 1000

const entry = (day: number, message: string, kind: HistoryEntrySummaryResponse['kind'], files: number, entered: number, left: number): HistoryEntrySummaryResponse => ({
  author: 'maya',
  file_change_count: files,
  id: `entry-${day}`,
  kind,
  message,
  occurred_at_unix: START + day * DAY,
  parent_id: null,
  source_id: `source-${day}`,
  visibility_summary: { entered_count: entered, left_count: left },
  views: null,
})

const entries = [
  entry(0, 'Initial import', 'push', 1840, 4, 0),
  entry(3, 'Hide internal docs', 'visibility_change', 22, 0, 22),
  entry(9, 'Publish API reference', 'visibility_change', 31, 31, 0),
  entry(17, 'Reconcile GitHub and Scope main histories', 'push', 441, 126, 0),
  entry(25, 'Hide scratch migrations', 'push', 210, 92, 18),
  entry(38, 'Publish examples and templates', 'visibility_change', 282, 282, 0),
].reverse()

export function useHistoryFeed() {
  return { resource: { status: 'loaded', error: null, retry: () => {}, value: { entries } } }
}
