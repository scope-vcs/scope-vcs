import type { RepoParams } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { HistoryFeedState } from '@/features/history/history-feed-state'
import { useHistoryFeed } from '@/features/history/history-feed'
import { useRepositorySearch } from '@/features/repo-search/repository-search-context'
import { VisibilityTimeline } from '@/features/history/visibility-timeline'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { viewingAsSearch } from '@/features/repo-detail/viewing-as'
import { Eye, History } from 'lucide-react'
import type { ViewId } from '@/api/types.generated'

export function VisibilityLogSection({ params, view }: { params: RepoParams; view: ViewId }) {
  const { repo } = useRepoLayout()
  const history = useHistoryFeed({ view, feed: 'visibility', params })
  const search = useRepositorySearch()
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
                search={viewingAsSearch(view, repo.access.view)}
                view={view}
              />
              <div className="flex justify-end">
                <Button onClick={() => search.open('visibility')} size="sm" type="button" variant="ghost">
                  <History />
                  Visibility history
                </Button>
              </div>
            </div>
          )}
        </HistoryFeedState>
      </SectionRow>
    </SectionRows>
  )
}
