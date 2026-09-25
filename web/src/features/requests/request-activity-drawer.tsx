import { PendingSurface } from '@/components/pending-surface'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
} from '@/components/ui/skeleton'
import { Link } from '@tanstack/react-router'
import { ChevronRight, History, TriangleAlert } from 'lucide-react'
import type { RefObject } from 'react'
import { REQUEST_ACTIVITY_PAGE_SIZE } from './request-discussion-api'
import { eventKindLabel, requestEventBody } from './request-labels'
import { RelativeTimestamp } from '@/components/timestamp'
import { RequestSideDrawer } from './request-side-drawer'
import type { RequestActivityPage } from './request-discussion-types'
import { actorHandle } from './request-actor'

const PENDING_ACTIVITY: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
  { id: 'fifth', length: 'long' },
]

export function RequestActivityDrawer({
  activity,
  error,
  loading,
  load,
  onOpenChange,
  open,
  params,
  returnFocus,
}: {
  activity: RequestActivityPage | null
  error: string | null
  loading: boolean
  load: () => void
  onOpenChange: (open: boolean) => void
  open: boolean
  params: { owner: string; repo: string; requestId: string }
  returnFocus: RefObject<HTMLElement | null>
}) {
  const events = activity
    ? [...activity.events].reverse()
    : []

  return (
    <RequestSideDrawer
      description="Durable workflow changes, newest first."
      footer={activity?.events.length === REQUEST_ACTIVITY_PAGE_SIZE ? (
        <p className="border-t border-border px-5 py-3 text-xs text-muted-foreground">
          Showing the latest {REQUEST_ACTIVITY_PAGE_SIZE} events.
        </p>
      ) : null}
      icon={<History />}
      onOpenChange={onOpenChange}
      open={open}
      returnFocus={returnFocus}
      title="Request history"
    >
      {error && activity ? (
        <div className="flex items-center gap-3 border-b border-border px-5 py-3 text-sm" role="alert">
          <span>{error}</span>
          <Button onClick={load} size="sm" variant="secondary">Retry</Button>
        </div>
      ) : null}
      {loading && !activity ? (
        <PendingSurface
          className="min-h-full"
          label="Loading request history"
          onRetry={load}
        >
          <RequestActivitySkeleton />
        </PendingSurface>
      ) : error && !activity ? (
        <div
          className="flex items-start gap-3 px-5 py-8 text-sm"
          role="alert"
        >
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-destructive" />
          <div className="grid gap-3">
            <p className="text-destructive">{error}</p>
            <div>
              <Button
                onClick={load}
                size="sm"
                type="button"
                variant="secondary"
              >
                Retry
              </Button>
            </div>
          </div>
        </div>
      ) : events.length > 0 ? (
        events.map((event) => {
          const body = requestEventBody(event)
          return (
            <article
              className="scope-content-enter grid gap-2 border-b border-border px-5 py-4"
              key={event.id}
            >
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{eventKindLabel(event.kind)}</Badge>
                <RelativeTimestamp
                  className="text-xs text-muted-foreground"
                  value={event.created_at_unix}
                />
                <span className="text-xs text-muted-foreground">
                  {actorHandle(event.actor)}
                </span>
                {event.kind === 'RevisionPushed' ? (
                  <Link
                    className="ml-auto flex items-center gap-1 rounded text-xs font-medium hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                    params={params}
                    search={{ revision: event.id }}
                    to="/$owner/$repo/requests/$requestId/changes"
                  >
                    View changes <ChevronRight aria-hidden="true" className="size-3.5" />
                  </Link>
                ) : null}
              </div>
              {body ? (
                <p className="whitespace-pre-wrap text-sm leading-6">
                  {body}
                </p>
              ) : null}
            </article>
          )
        })
      ) : (
        <p className="px-5 py-8 text-sm text-muted-foreground">
          No request history yet.
        </p>
      )}
    </RequestSideDrawer>
  )
}

function RequestActivitySkeleton() {
  return (
    <div className="divide-y divide-border">
      {PENDING_ACTIVITY.map((event) => (
        <div className="grid gap-3 px-5 py-4" key={event.id}>
          <div className="flex items-center gap-2">
            <BlockSkeleton className="h-5 w-20 rounded-full" />
            <TextSkeleton length="short" size="meta" />
            <TextSkeleton length="tiny" size="meta" />
          </div>
          <LineSkeleton length={event.length} />
        </div>
      ))}
    </div>
  )
}
