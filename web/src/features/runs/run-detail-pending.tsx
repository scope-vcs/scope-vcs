import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { RUN_JOB_LIST_CLASS, RUN_JOB_ROW_CLASS } from './run-job-layout'
import { RunEnvironmentSummary } from './run-attempt-environment'
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
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <TextSkeleton length="medium" size="meta" />
          <div className="mt-2 flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <TextSkeleton length="long" size="heading" />
              <TextSkeleton className="mt-3" length="medium" size="meta" />
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <BlockSkeleton className="h-9 w-24" />
              <BlockSkeleton className="h-9 w-28" />
            </div>
          </div>
        </header>
        <div className="flex min-h-0 flex-1 flex-col border-t border-border lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)]">
          <div className="min-w-0 border-b border-border lg:border-b-0 lg:border-r">
            <div className="flex items-center justify-between gap-2 px-4 pt-3">
              <span className="text-sm font-semibold">Jobs</span>
              <BlockSkeleton className="h-8 w-16" />
            </div>
            <div className="px-4">
              <TextSkeleton length="short" size="meta" />
            </div>
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
            <section className="border-b border-border">
              <RunEnvironmentSummary
                cacheSummary={<TextSkeleton className="inline-block align-middle" length="medium" size="meta" />}
                image={<TextSkeleton className="inline-block align-middle" length="short" size="meta" />}
              />
            </section>
            {PENDING_STEPS.map((step) => (
              <div className={`${RUN_STEP_ROW_CLASS} border-b border-border`} key={step.id}>
                <BlockSkeleton className="size-3.5 rounded-full" />
                <span className="min-w-0">
                  <TextSkeleton length={step.length} />
                  <TextSkeleton className="mt-0.5" length="medium" size="meta" />
                </span>
                <TextSkeleton length="tiny" size="meta" />
              </div>
            ))}
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
