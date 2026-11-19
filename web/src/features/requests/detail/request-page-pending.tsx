import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { LineSkeleton, type LineSkeletonLength } from '@/components/ui/line-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { REQUEST_DISCUSSION_CONTENT_CLASS } from './request-content-layout'
import { RequestChecksPending } from '../checks/request-checks-pending'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestChangesBodyPending } from '../changes/request-changes-body-pending'
import { RequestChangesScreen } from '../changes/request-changes-screen'
import './request-detail.css'

const PENDING_THREADS: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
]

export function RequestDetailPagePending() {
  const { isSignedIn } = useAuth()
  return (
    <PendingSurface label="Loading request">
      <div className="w-full">
        <header className="request-detail-header px-(--app-inset) pb-2 pt-3 min-[701px]:pt-6">
          <TextSkeleton className="mb-3 py-1 min-[701px]:hidden" length="short" size="meta" />
          <TextSkeleton length="xlong" size="heading" />
          <div className="request-detail-header-state mt-4">
            <BlockSkeleton className="h-6 w-40 rounded-md" />
            {isSignedIn ? <BlockSkeleton className="hidden h-7 w-20 min-[701px]:block" /> : null}
          </div>
          <div className="request-detail-header-facts mt-2">
            <TextSkeleton className="h-5 py-1" length="long" size="meta" />
            <div className="request-detail-header-actions">
              <BlockSkeleton className="h-7 w-44" />
              <BlockSkeleton className="h-7 w-24" />
              <BlockSkeleton className="size-7" />
            </div>
          </div>
        </header>
        <RequestChecksPending />
        <div className="min-w-0 pt-4">
          <section className="min-w-0 border-b border-border px-(--app-inset) pb-5">
            <TextSkeleton className="h-6 py-1" length="xlong" />
          </section>
          <div className="min-w-0">
            <ChildRoutesPending
              below="/$owner/$repo/requests/$requestId/_discussion"
              fallback={<DiscussionSkeleton />}
            />
          </div>
        </div>
        {isSignedIn ? (
          <div className="fixed inset-x-0 bottom-0 z-30 flex justify-end gap-2 border-t border-border bg-background px-(--app-inset) py-3 min-[701px]:hidden">
            <BlockSkeleton className="h-7 w-20" />
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

function DiscussionSkeleton() {
  return (
    <section className="divide-y divide-border px-(--app-inset)">
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
