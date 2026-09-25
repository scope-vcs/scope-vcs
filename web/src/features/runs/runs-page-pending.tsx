import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import {
  RUN_ROW_CLASS,
  RUN_ROW_DURATION_CLASS,
  RUN_ROW_PRIMARY_CLASS,
  RUN_ROW_TIMESTAMP_CLASS,
} from './run-row-layout'

const RUN_LENGTHS: TextSkeletonLength[] = ['medium', 'short', 'long', 'medium', 'medium']
// Enough rows to fill a first screen, so dividers line up with any list length.
const PENDING_RUNS = Array.from({ length: 16 }, (_, row) => ({
  id: `run-${row}`,
  length: RUN_LENGTHS[row % RUN_LENGTHS.length],
}))
const PENDING_ACTIONS = (
  <div className="flex items-center gap-2">
    <BlockSkeleton className="h-8 w-36" />
    <BlockSkeleton className="h-8 w-28" />
  </div>
)

export function RunsPagePending() {
  return (
    <PendingSurface label="Loading runs">
      <WorkbenchPane>
        <WorkbenchBar actions={PENDING_ACTIONS} title="Runs" />
        <div className="min-w-0 border-t border-border">
          <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
            <div className="divide-y divide-border pt-7">
              {PENDING_RUNS.map((run) => (
                <div className={RUN_ROW_CLASS} key={run.id}>
                  <BlockSkeleton className="size-3.5 shrink-0 rounded-full" />
                  <div className={RUN_ROW_PRIMARY_CLASS}>
                    {/* Below sm the loaded name and hash share one inline line
                        box, which is taller than a text-sm line. */}
                    <TextSkeleton className="h-6 py-1 sm:h-5 sm:py-0.5" length={run.length} />
                    <TextSkeleton
                      className="hidden sm:block"
                      length="short"
                      size="meta"
                    />
                  </div>
                  <div className={RUN_ROW_DURATION_CLASS}>
                    <TextSkeleton className="ml-auto" length="tiny" size="meta" />
                  </div>
                  <div className={RUN_ROW_TIMESTAMP_CLASS}>
                    <TextSkeleton className="ml-auto" length="short" size="meta" />
                  </div>
                </div>
              ))}
            </div>
          </main>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
