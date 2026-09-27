import type { RepoParams } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { HistoryFeedList } from '@/features/history/history-entry-list'
import { useHistoryFeed } from '@/features/history/history-feed'
import { Eye } from 'lucide-react'

// Visibility changes are not Git history, so this is their maintainer record.
export function VisibilityLogSection({ params }: { params: RepoParams }) {
  const history = useHistoryFeed({ audience: 'private', feed: 'visibility', params })
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
            search={{}}
          />
        </div>
      </SectionRow>
    </SectionRows>
  )
}
