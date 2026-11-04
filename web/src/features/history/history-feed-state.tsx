import type { RepoParams } from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { MENU_LIST_ROW_CLASS } from '@/components/menu-list-panel'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { historyEntryLabels } from '@/features/history/history-row-labels'
import { Link } from '@tanstack/react-router'
import { LoaderCircle } from 'lucide-react'
import type { ReactNode } from 'react'
import type { HistoryEntrySummaryResponse, ViewId } from '@/api/types.generated'
import type { useHistoryFeed } from './history-feed'
import { useRepoLayout, useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { viewingAsSearch } from '@/features/repo-detail/viewing-as'

type HistoryFeedResource = ReturnType<typeof useHistoryFeed>['resource']
type HistoryFeedPage = NonNullable<HistoryFeedResource['value']>

export function HistoryFeedState({
  children,
  empty,
  resource,
}: {
  children: (page: HistoryFeedPage) => ReactNode
  empty: string
  resource: HistoryFeedResource
}) {
  if (resource.status === 'failed') {
    return (
      <PanelState tone="error">
        <span>{resource.error}</span>
        <Button onClick={resource.retry} size="sm" variant="secondary">Retry</Button>
      </PanelState>
    )
  }
  if (resource.status !== 'loaded') {
    return (
      <PendingSurface label="Loading history" onRetry={resource.retry}>
        <div className="grid gap-4 p-3">
          <TextSkeleton length="long" />
          <TextSkeleton length="medium" />
          <TextSkeleton length="long" />
        </div>
      </PendingSurface>
    )
  }
  if (!resource.value.entries.length) {
    return <p className="px-3 py-6 text-center text-xs text-muted-foreground">{empty}</p>
  }
  return children(resource.value)
}
