import type { RepoParams } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { HistoryFeedList } from '@/features/history/history-entry-list'
import { useHistoryFeed } from '@/features/history/history-feed'
import { Eye } from 'lucide-react'
import type { ViewId } from '@/api/types.generated'

export function VisibilityLogSection({ params, view }: { params: RepoParams; view: ViewId }) {
  const history = useHistoryFeed({ view, feed: 'visibility', params })
  return (
    <SectionRows>
      <SectionRow
        description="Who changed which paths' visibility, newest first. Includes changes made as part of a push."
        icon={<Eye className="size-4" />}
        title="Visibility changes"
      >
        <div className="-mx-3">
          <HistoryFeedList
            empty="No visibility changes yet."
            history={history}
            params={params}
          />
        </div>
      </SectionRow>
    </SectionRows>
  )
}
