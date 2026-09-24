import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { TextSkeleton } from '@/components/ui/skeleton'
import { useSearch } from '@tanstack/react-router'
import { parseHistoryFeed } from '@/api/history-inputs'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'
import { HistoryEntryListSkeleton } from './history-entry-list'
import { HistoryFeedTogglePending } from './history-feeds'

// Mirrors HistoryPage: the feed toggle, the entry list box, then the selected
// entry's detail, which its panel draws the same way while it loads.
export function HistoryPagePending() {
  const feed = useSearch({ strict: false, select: (search) => parseHistoryFeed(search.feed) })
  return (
    <PendingSurface label="Loading repository history">
      <WorkbenchPane>
        <WorkbenchBar summary={<TextSkeleton length="short" />} title="history" />
        <section className="border-t border-border">
          <HistoryFeedTogglePending feed={feed} />
          <div className="max-h-80 overflow-hidden border-b border-border">
            <HistoryEntryListSkeleton />
          </div>
          <CommitDetailSkeleton />
        </section>
      </WorkbenchPane>
    </PendingSurface>
  )
}
