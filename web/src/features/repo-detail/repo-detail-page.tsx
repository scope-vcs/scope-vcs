import type { RepoContent, RepoParams } from '@/api/types'
import type {
  RepoFileContentResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { Eye } from 'lucide-react'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { RepositoryCodeView } from './repository-code-view'
import { useRepositoryDependencyCheck } from './repository-dependency-check'
import { RepositoryDependencyLeak } from './repository-dependency-check-view'
import { RepositoryDependencyStatus } from './repository-dependency-status'
import { RepositoryLatestActivity } from './repository-latest-activity'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { displayRouteFilePath } from '@/lib/route-file'
import type { CachedResource } from '@/lib/use-cached-resource'
import { useRepoViews } from './repo-layout-context'
import { RepoViewingAsPicker, useViewingAs } from './use-viewing-as'

export function RepoDetailPage({
  content,
  file,
  onSelectFilePath,
  params,
  repo,
  selectedPath,
}: {
  content: CachedResource<RepoContent>
  file: CachedResource<RepoFileContentResponse>
  onSelectFilePath: (path: string) => void
  params: RepoParams
  repo: RepoSummaryResponse
  selectedPath: string | null
}) {
  const workspaceTabs = useWorkspaceTabs({ activeId: selectedPath })
  const { options, view } = useViewingAs()
  const views = useRepoViews()
  const dependencyCheck = useRepositoryDependencyCheck({ params, repo })

  function selectResource(path: string) {
    workspaceTabs.open(displayRouteFilePath(path), false)
    onSelectFilePath(path)
  }

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            <RepositoryDependencyStatus
              onSelectFilePath={selectResource}
              presentation={dependencyCheck}
              views={views}
            />
            {options.length > 1 ? <RepoViewingAsPicker /> : (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Eye aria-hidden="true" className="size-3.5" />
                Viewing as {views.name(view)}
              </span>
            )}
            {repo.lifecycle_state === 'Ready' && (content.value ? (
              <RepoCloneDropdown
                cloneRemoteUrl={content.value.clone_remote_url}
                repo={repo}
                view={view}
                viewName={views.name(view)}
              />
            ) : content.status === 'loading' && <BlockSkeleton className="h-8 w-24" />)}
            <RepoPrimaryActionButton
              includeOpen={false}
              repo={repo}
              requireOwner
              variant="default"
            />
          </>
        )}
        summary={<RepositoryLatestActivity params={params} repo={repo} />}
        title="Code"
      />
      <RepositoryDependencyLeak
        onSelectFilePath={selectResource}
        presentation={dependencyCheck}
        views={views}
      />
      <RepositoryCodeView
        content={content}
        file={file}
        onSelectFilePath={onSelectFilePath}
        params={params}
        repo={repo}
        selectedPath={selectedPath}
        workspaceTabs={workspaceTabs}
      />
    </WorkbenchPane>
  )
}
