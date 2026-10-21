import { WorkbenchBar, WorkbenchBody, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton, type TextSkeletonLength } from '@/components/ui/text-skeleton'
import { cn } from '@/lib/utils'
import {
  RUN_HISTORY_ROW_CLASS,
  RUN_HISTORY_ROW_ICON_CLASS,
  RUN_HISTORY_ROW_PRIMARY_CLASS,
  RUN_ROW_DURATION_CLASS,
  RUN_ROW_TIMESTAMP_CLASS,
  RUN_ROW_WIDE_COLUMN_CLASS,
} from './run-layout'

const RUN_LENGTHS: TextSkeletonLength[] = ['medium', 'short', 'long', 'medium', 'medium']
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
        <WorkbenchBody>
          <div className="divide-y divide-border pt-7">
            {PENDING_RUNS.map((run) => (
              <div className={RUN_HISTORY_ROW_CLASS} key={run.id}>
                <BlockSkeleton className={cn(RUN_HISTORY_ROW_ICON_CLASS, 'size-3.5 rounded-full')} />
                <div className={RUN_HISTORY_ROW_PRIMARY_CLASS}>
                  <TextSkeleton className="col-span-3 col-start-2 row-start-1" length={run.length} />
                  <TextSkeleton className="col-span-3 col-start-2 row-start-2" length="short" size="meta" />
                </div>
                <div className={cn(RUN_ROW_DURATION_CLASS, RUN_ROW_WIDE_COLUMN_CLASS)}>
                  <TextSkeleton className="ml-auto" length="tiny" size="meta" />
                </div>
                <div className={cn(RUN_ROW_TIMESTAMP_CLASS, RUN_ROW_WIDE_COLUMN_CLASS)}>
                  <TextSkeleton className="ml-auto" length="short" size="meta" />
                </div>
              </div>
            ))}
          </div>
        </WorkbenchBody>
      </WorkbenchPane>
    </PendingSurface>
  )
}
