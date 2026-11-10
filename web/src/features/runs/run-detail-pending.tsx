import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton, type TextSkeletonLength } from '@/components/ui/text-skeleton'
import { RUN_BODY_CLASS } from './run-layout'

const PENDING_JOBS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'short' },
  { id: 'second', length: 'tiny' },
  { id: 'third', length: 'medium' },
]
const PENDING_STEPS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'long' },
  { id: 'third', length: 'short' },
  { id: 'fourth', length: 'long' },
  { id: 'fifth', length: 'medium' },
]

export function RunDetailPagePending() {
  return (
    <PendingSurface label="Loading run details">
      <WorkbenchPane className="flex flex-col lg:h-[calc(100dvh-var(--app-topbar))]">
        <header className="flex items-center gap-4 px-4 py-3">
          <div className="min-w-0 flex-1">
            <TextSkeleton length="tiny" size="meta" />
            <TextSkeleton className="mt-1.5" length="short" />
            <TextSkeleton className="mt-1.5" length="medium" size="meta" />
          </div>
          <BlockSkeleton className="h-8 w-28" />
        </header>
        <div className={RUN_BODY_CLASS}>
          <div className="min-w-0 border-b border-border p-2 lg:border-b-0 lg:border-r lg:px-0 lg:py-2">
            <BlockSkeleton className="h-9 w-full lg:hidden" />
            <div className="hidden lg:block">
              {PENDING_JOBS.map((job) => (
                <div className="flex items-center gap-2 py-1.5 pl-3 pr-3" key={job.id}>
                  <BlockSkeleton className="size-3.5 rounded-full" />
                  <span className="min-w-0 flex-1">
                    <TextSkeleton length={job.length} />
                  </span>
                  <TextSkeleton length="tiny" size="meta" />
                </div>
              ))}
            </div>
          </div>
          <div className="min-w-0">
            <div className="flex min-h-12 items-center gap-2.5 border-b border-border py-2 pl-4 pr-3">
              <BlockSkeleton className="size-3.5 rounded-full" />
              <TextSkeleton length="short" />
            </div>
            {PENDING_STEPS.map((step) => (
              <div className="grid h-9 grid-cols-[0.875rem_0.875rem_minmax(0,1fr)_auto] items-center gap-2.5 border-b border-border px-4" key={step.id}>
                <span className="size-3.5" />
                <BlockSkeleton className="size-3.5 rounded-full" />
                <TextSkeleton length={step.length} />
                <TextSkeleton length="tiny" size="meta" />
              </div>
            ))}
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
