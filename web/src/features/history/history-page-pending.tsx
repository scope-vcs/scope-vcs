import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'

const PENDING_ACTIONS = <BlockSkeleton className="h-8 w-28" />
const PENDING_SUMMARY = <TextSkeleton length="short" />

export function HistoryPagePending() {
  return (
    <PendingSurface label="Loading repository history">
      <WorkbenchPane>
        <WorkbenchBar actions={PENDING_ACTIONS} summary={PENDING_SUMMARY} title="history" />
        <div className="min-w-0 border-t border-border">
          <div className="border-b border-border px-5 py-3">
            <TextSkeleton length="short" />
          </div>
          <CommitDetailSkeleton showDiff />
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
