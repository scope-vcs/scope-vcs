import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { useRef } from 'react'
import { REQUEST_DISCUSSION_CONTENT_CLASS } from './request-content-layout'
import { RequestChecksPending } from './request-checks-pending'
import { RequestDetailsSkeleton } from './request-details-layout'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestChangesScreen } from './request-changes-screen'
import { useDetailPaneRail } from './use-detail-pane-rail'
import { DiffSkeleton } from '@/features/review/diff-skeleton'

const PENDING_THREADS: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
]
const PENDING_CHANGES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
]

// Mirrors RequestDetailPage: the same header, checks row, description, and
// details rail when the pane is wide enough for one.
export function RequestDetailPagePending() {
  const { isSignedIn } = useAuth()
  const paneRef = useRef<HTMLDivElement>(null)
  const rail = useDetailPaneRail(paneRef)
  return (
    <PendingSurface label="Loading request">
      <div className="request-detail-pane w-full" ref={paneRef}>
        <header className="request-detail-header border-b border-border px-5 pb-4 pt-6 sm:px-6 lg:px-8">
          <TextSkeleton length="xlong" size="heading" />
          <div className="request-detail-header-secondary mt-4">
            <div className="request-detail-header-meta flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 text-xs leading-5">
              {/* Mergeability badge, branch, then author: the same pieces as
                  the loaded row, so they wrap the same way. */}
              <BlockSkeleton className="h-5 w-40 rounded-md" />
              {/* The branch row is as tall as its copy button. */}
              <TextSkeleton className="h-6 py-1.5" length="long" size="meta" />
              <TextSkeleton className="h-5 py-1" length="medium" size="meta" />
            </div>
            {/* Everyone gets Changes, and Details when there is no rail.
                Signed-in viewers also get the one lifecycle action, which
                moves to a bottom bar on narrow panes, and the More menu. */}
            <div className="request-detail-header-actions flex min-w-0 items-center justify-end gap-2">
              <BlockSkeleton className="h-8 w-[6.5rem]" />
              {rail ? null : <BlockSkeleton className="h-8 w-24" />}
              {isSignedIn ? (
                <>
                  <BlockSkeleton className="hidden h-8 w-20 min-[701px]:block" />
                  <BlockSkeleton className="size-8" />
                </>
              ) : null}
            </div>
          </div>
        </header>
        <div className="request-detail-actions px-5 py-2.5 min-[701px]:hidden">
          <BlockSkeleton className="size-8" />
        </div>
        <RequestChecksPending />
        <div className={cn(rail && 'grid grid-cols-[minmax(0,1fr)_300px]')}>
          <div className="request-detail-document pt-4">
            <section className="min-w-0 border-b border-border px-5 pb-5 lg:px-7">
              {/* Descriptions are set at leading-6. */}
              <TextSkeleton className="h-6 py-1" length="xlong" />
            </section>
            <div className="min-w-0">
              <ChildRoutesPending
                below="/$owner/$repo/requests/$requestId/_discussion"
                fallback={<DiscussionSkeleton />}
              />
            </div>
          </div>
          {rail ? (
            <aside className="min-w-0 border-l border-border">
              <RequestDetailsSkeleton />
            </aside>
          ) : null}
        </div>
        {isSignedIn ? (
          <div className="fixed inset-x-0 bottom-0 z-30 flex justify-end gap-2 border-t border-border bg-background px-3 py-3 min-[701px]:hidden">
            <BlockSkeleton className="h-8 w-20" />
          </div>
        ) : null}
      </div>
    </PendingSurface>
  )
}

export function RequestDiscussionPending() {
  return (
    <PendingSurface label="Loading request discussion">
      <DiscussionSkeleton />
    </PendingSurface>
  )
}

export function RequestChangesPending() {
  const params = useParams({ from: '/$owner/$repo/requests/$requestId' })
  return (
    <RequestChangesScreen
      params={params}
      revisions={null}
      selectedRevisionId={null}
      title={<TextSkeleton length="long" size="meta" />}
    >
      <RequestChangesBodyPending />
    </RequestChangesScreen>
  )
}

export function RequestChangesBodyPending() {
  return (
    <PendingSurface label="Loading request changes">
      <section className="grid border-t border-border lg:grid-cols-[minmax(220px,0.42fr)_minmax(0,1.58fr)]">
        <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
          {PENDING_CHANGES.map((change) => (
            <div className="px-5 py-4" key={change.id}>
              <TextSkeleton length={change.length} />
              <TextSkeleton className="mt-2" length="short" size="meta" />
            </div>
          ))}
        </div>
        <div className="min-h-[340px] p-5 lg:p-6">
          <TextSkeleton length="long" />
          <TextSkeleton className="mt-2" length="medium" size="meta" />
          <div className="mt-4">
            <DiffSkeleton />
          </div>
        </div>
      </section>
    </PendingSurface>
  )
}

function DiscussionSkeleton() {
  return (
    <section className="divide-y divide-border px-5 lg:px-7">
      {PENDING_THREADS.map((thread) => (
        <article className="py-5" key={thread.id}>
          <div className="flex items-center gap-2">
            <BlockSkeleton className="size-7 rounded-full" />
            <TextSkeleton length="short" size="meta" />
          </div>
          <div className={`mt-4 space-y-2 ${REQUEST_DISCUSSION_CONTENT_CLASS}`}>
            <LineSkeleton length={thread.length} />
            <LineSkeleton length="medium" />
          </div>
        </article>
      ))}
    </section>
  )
}
