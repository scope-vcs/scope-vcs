import type { RepoParams } from '@/api/types'
import type { HistoryEntrySummaryResponse, ViewId } from '@/api/types.generated'
import { formatUnixMonthDay } from '@/lib/date-format'
import { useHydrated } from '@/lib/use-hydrated'
import { barY, defineChart, ruleY, text, type ChartMarkStateContext } from '@tanstack/charts'
import { decorative } from '@tanstack/charts/mark/decorative'
import { Chart } from '@tanstack/charts/react'
import { scaleBand } from '@tanstack/charts/scales/band'
import { useNavigate } from '@tanstack/react-router'
import { scaleSqrt } from 'd3-scale'
import { useMemo, useState } from 'react'
import { historyCommitTitle, historyEntryCountLabel, historyEntryKindLabel } from './history-row-labels'
import type { RepoViews } from '@/api/repo-views'
import { useRepoViews } from '@/features/repo-detail/repo-layout-context'
import type { ViewingAsSearch } from '@/features/repo-detail/viewing-as'
import {
  VISIBILITY_TIMELINE_ENTRY_LIMIT,
  visibilityTimelineBars,
  type VisibilityDirection,
  type VisibilityTimelineBar,
} from './visibility-timeline-rows'

const BAR_FILL: Record<VisibilityDirection, string> = {
  entered: 'var(--success)',
  left: 'var(--border-strong)',
}

const LABEL_FILL = 'var(--foreground)'

const isFocusedEntry = ({ datum, focus }: ChartMarkStateContext<VisibilityTimelineBar>) => datum.entry === focus.primary.datum.entry
const isOtherEntry = (context: ChartMarkStateContext<VisibilityTimelineBar>) => !isFocusedEntry(context)

export function VisibilityTimeline({
  entries,
  params,
  search,
  view,
}: {
  entries: readonly HistoryEntrySummaryResponse[]
  params: RepoParams
  search: ViewingAsSearch
  view: ViewId
}) {
  const hydrated = useHydrated()
  const navigate = useNavigate()
  const views = useRepoViews()
  const [focusedEntryId, setFocusedEntryId] = useState<string | null>(null)
  const definition = useMemo(
    () => visibilityTimelineDefinition(visibilityTimelineBars(entries), hydrated),
    [entries, hydrated],
  )
  const shown = Math.min(entries.length, VISIBILITY_TIMELINE_ENTRY_LIMIT)
  const focusedEntry = entries.find((entry) => entry.id === focusedEntryId)
  const detailEntry = focusedEntry ?? entries[0]

  return (
    <div className="grid gap-2">
      <Chart
        ariaLabel={`${shown} most recent visibility ${shown === 1 ? 'change' : 'changes'}, oldest to newest`}
        className="text-muted-foreground"
        definition={definition}
        height={136}
        onFocusChange={(point) => setFocusedEntryId(point?.datum.entry.id ?? null)}
        onSelect={(point) => {
          if (!point) return
          void navigate({
            params: { ...params, entryId: point.datum.entry.source_id },
            search,
            to: '/$owner/$repo/updates/$entryId',
          })
        }}
      />
      {detailEntry && (
        <TimelineEntryDetail entry={detailEntry} hydrated={hydrated} latest={!focusedEntry} view={view} views={views} />
      )}
    </div>
  )
}

function visibilityTimelineDefinition(bars: readonly VisibilityTimelineBar[], hydrated: boolean) {
  const dateLabels = new Map(bars.flatMap(({ entry }) => entry.occurred_at_unix === null
    ? []
    : [[entry.id, formatUnixMonthDay(entry.occurred_at_unix, hydrated)] as const]))
  const counts = bars.map((bar) => bar.signedCount)
  const entryId = (bar: VisibilityTimelineBar) => bar.entry.id
  const barKey = (bar: VisibilityTimelineBar) => `${bar.entry.id}:${bar.direction}`

  return defineChart({
    marks: [
      decorative(ruleY([0], { stroke: 'var(--border-strong)' })),
      barY(bars, {
        fill: (bar) => BAR_FILL[bar.direction],
        key: barKey,
        maxThickness: 28,
        radius: { end: 4 },
        states: [{ style: { opacity: 0.32 }, when: isOtherEntry }],
        x: entryId,
        y: 'signedCount',
      }),
      decorative(text(bars, {
        dy: (bar) => bar.signedCount < 0 ? 10 : -8,
        fill: (bar) => bar.labeled ? LABEL_FILL : 'transparent',
        fontSize: 11,
        fontWeight: 500,
        key: barKey,
        states: [
          { style: { fill: LABEL_FILL }, when: isFocusedEntry },
          { style: { fill: 'transparent' }, when: isOtherEntry },
        ],
        text: (bar) => Math.abs(bar.signedCount),
        x: entryId,
        y: 'signedCount',
      })),
    ],
    scales: {
      x: {
        axis: {
          line: false,
          ticks: { format: (id: string) => dateLabels.get(id) ?? '', size: 0, values: [...dateLabels.keys()] },
        },
        scale: () => scaleBand<string>().padding(0.28),
      },
      y: {
        axis: false,
        scale: scaleSqrt().domain([Math.min(0, ...counts), Math.max(0, ...counts)]),
      },
    },
    focus: 'nearest-x',
    focusRing: false,
  })
}

function TimelineEntryDetail({
  entry,
  hydrated,
  latest,
  view,
  views,
}: {
  entry: HistoryEntrySummaryResponse
  hydrated: boolean
  latest: boolean
  view: ViewId
  views: RepoViews
}) {
  return (
    <div className="grid gap-0.5 border-t border-border pt-2.5 text-xs text-muted-foreground">
      <div className="flex items-baseline gap-1.5">
        {latest && <span className="shrink-0 rounded border border-border px-1 text-[11px]">Latest</span>}
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground">{historyCommitTitle(entry)}</span>
        {entry.occurred_at_unix !== null && (
          <span className="ml-1.5 shrink-0 tabular-nums">{formatUnixMonthDay(entry.occurred_at_unix, hydrated)}</span>
        )}
      </div>
      <span className="truncate">{[historyEntryKindLabel(entry.kind), entry.author].filter(Boolean).join(' · ')}</span>
      <span className="truncate tabular-nums">{historyEntryCountLabel(entry, view, views)}</span>
    </div>
  )
}
