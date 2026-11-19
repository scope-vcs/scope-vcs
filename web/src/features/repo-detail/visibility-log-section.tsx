import type { RepoParams } from '@/api/types'
import { HistoryFeedState } from '@/features/history/history-feed-state'
import { useHistoryFeed } from '@/features/history/history-feed'
import { useRepositorySearch } from '@/features/repo-search/repository-search-context'
import { VisibilityTimeline } from '@/features/history/visibility-timeline'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { viewingAsSearch } from '@/features/repo-detail/viewing-as'
import { VisibilityChangesSection } from '@/features/repo-detail/repo-settings-sections'
import type { ViewId } from '@/api/types.generated'

export function VisibilityLogSection({ params, view }: { params: RepoParams; view: ViewId }) {
  const { repo } = useRepoLayout()
  const history = useHistoryFeed({ view, feed: 'visibility', params })
  const search = useRepositorySearch()
  return (
    <VisibilityChangesSection onOpenHistory={() => search.open('visibility')}>
      <HistoryFeedState empty="No visibility changes yet." resource={history.resource}>
        {(page) => (
          <VisibilityTimeline
            entries={page.entries}
            params={params}
            search={viewingAsSearch(view, repo.access.view)}
            view={view}
          />
        )}
      </HistoryFeedState>
    </VisibilityChangesSection>
  )
}
