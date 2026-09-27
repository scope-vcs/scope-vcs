import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { RUN_JOB_LIST_CLASS, RUN_JOB_ROW_CLASS } from './run-job-layout'
import { RUN_STEP_ROW_CLASS } from './run-step-layout'

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
            <TextSkeleton length="short" />
            <TextSkeleton className="mt-1.5" length="medium" size="meta" />
          </div>
          <BlockSkeleton className="h-8 w-24" />
        </header>
        <div className="flex min-h-0 flex-1 flex-col border-t border-border lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)]">
          <div className="min-w-0 border-b border-border lg:border-b-0 lg:border-r lg:py-2">
            <div className={RUN_JOB_LIST_CLASS}>
              {PENDING_JOBS.map((job) => (
                <div className={RUN_JOB_ROW_CLASS} key={job.id}>
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
              <div className={`${RUN_STEP_ROW_CLASS} border-b border-border`} key={step.id}>
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
