import { PanelState } from '@/components/empty-state'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import type { ReactNode } from 'react'
import type { useHistoryFeed } from './history-feed'

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
