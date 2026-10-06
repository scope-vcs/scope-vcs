import type { RepoParams } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { HistoryFeedState } from '@/features/history/history-entry-list'
import { useHistoryFeed } from '@/features/history/history-feed'
import { HistoryMenu } from '@/features/history/history-menu'
import { updateViewSearch } from '@/features/history/update-search'
import { VisibilityTimeline } from '@/features/history/visibility-timeline'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { Eye } from 'lucide-react'

export function VisibilityLogSection({ params }: { params: RepoParams }) {
  const { repo } = useRepoLayout()
  const history = useHistoryFeed({ view: 'private', feed: 'visibility', params })
  return (
    <SectionRows>
      <SectionRow
        description="How paths' visibility changed over time, including changes made by pushes."
        icon={<Eye className="size-4" />}
        title="Visibility changes"
      >
        <HistoryFeedState empty="No visibility changes yet." resource={history.resource}>
          {(page) => (
            <div className="grid gap-2">
              <VisibilityTimeline
                entries={page.entries}
                params={params}
                search={updateViewSearch('private', repo.access.view)}
              />
              <div className="flex justify-end text-xs">
                <HistoryMenu
                  access={repo.access}
                  initialFeed="visibility"
                  label="Visibility history"
                  params={params}
                />
              </div>
            </div>
          )}
        </HistoryFeedState>
      </SectionRow>
    </SectionRows>
  )
}
