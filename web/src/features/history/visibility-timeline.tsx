import type { RepoParams } from '@/api/types'
import type { HistoryEntrySummaryResponse, ViewId } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { formatUnixMonthDay } from '@/lib/date-format'
import { useHydrated } from '@/lib/use-hydrated'
import { barY, defineChart, ruleY, text } from '@tanstack/charts'
import { decorative } from '@tanstack/charts/mark/decorative'
import { Chart } from '@tanstack/charts/react/tooltip'
import { scaleBand } from '@tanstack/charts/scales/band'
import { tooltip } from '@tanstack/charts/tooltip'
import { useNavigate } from '@tanstack/react-router'
import { scaleSqrt } from 'd3-scale'
import { useMemo } from 'react'
import { historyCommitTitle, historyEntryCountLabel, historyEntryKindLabel } from './history-row-labels'
import type { UpdateSearch } from './update-search'
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

const CHART_CLASS = [
  'text-muted-foreground',
  '[--ts-chart-tooltip-background:var(--popover)]',
  '[--ts-chart-tooltip-color:var(--popover-foreground)]',
  '[--ts-chart-tooltip-border:1px_solid_var(--border)]',
  '[--ts-chart-tooltip-border-radius:8px]',
  '[--ts-chart-tooltip-shadow:var(--shadow-pop)]',
  '[--ts-chart-tooltip-max-width:18rem]',
].join(' ')

export function VisibilityTimeline({
  entries,
  params,
  search,
  view,
}: {
  entries: readonly HistoryEntrySummaryResponse[]
  params: RepoParams
  search: UpdateSearch
  view: ViewId
}) {
  const hydrated = useHydrated()
  const navigate = useNavigate()
  const definition = useMemo(
    () => visibilityTimelineDefinition(visibilityTimelineBars(entries), hydrated),
    [entries, hydrated],
  )
  const shown = Math.min(entries.length, VISIBILITY_TIMELINE_ENTRY_LIMIT)

  return (
    <Chart
      ariaLabel={`${shown} most recent visibility ${shown === 1 ? 'change' : 'changes'}, oldest to newest`}
      className={CHART_CLASS}
      definition={definition}
      height={136}
      onSelect={(point) => {
        if (!point) return
        void navigate({
          params: { ...params, entryId: point.datum.entry.source_id },
          search,
          to: '/$owner/$repo/updates/$entryId',
        })
      }}
      renderTooltipBody={({ primaryPoint }) => primaryPoint ? <TimelineTooltip entry={primaryPoint.datum.entry} view={view} /> : null}
    />
  )
}

function visibilityTimelineDefinition(bars: readonly VisibilityTimelineBar[], hydrated: boolean) {
  const dateLabels = new Map(bars.flatMap(({ entry }) => entry.occurred_at_unix === null
    ? []
    : [[entry.id, formatUnixMonthDay(entry.occurred_at_unix, hydrated)] as const]))
  const counts = bars.map((bar) => bar.signedCount)
  const entryId = (bar: VisibilityTimelineBar) => bar.entry.id

  return defineChart({
    marks: [
      decorative(ruleY([0], { stroke: 'var(--border-strong)' })),
      barY(bars, {
        fill: (bar) => BAR_FILL[bar.direction],
        key: (bar) => `${bar.entry.id}:${bar.direction}`,
        maxThickness: 28,
        radius: { end: 4 },
        x: entryId,
        y: 'signedCount',
      }),
      decorative(text(bars.filter((bar) => bar.label !== null), {
        dy: (bar) => bar.signedCount < 0 ? 10 : -8,
        fill: 'var(--foreground)',
        fontSize: 11,
        fontWeight: 500,
        text: 'label',
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
    tooltip,
  })
}

function TimelineTooltip({ entry, view }: { entry: HistoryEntrySummaryResponse; view: ViewId }) {
  return (
    <span className="grid gap-0.5 text-xs">
      <span className="text-[13px] font-medium text-foreground">{historyCommitTitle(entry)}</span>
      <span className="text-muted-foreground">
        {[historyEntryKindLabel(entry.kind), entry.author].filter(Boolean).join(' · ')}
        {entry.occurred_at_unix !== null ? <> · <RelativeTimestamp value={entry.occurred_at_unix} /></> : null}
      </span>
      <span className="tabular-nums text-muted-foreground">{historyEntryCountLabel(entry, view)}</span>
    </span>
  )
}
