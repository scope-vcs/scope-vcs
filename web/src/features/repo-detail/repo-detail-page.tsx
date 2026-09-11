import type { RepoContent, RepoParams } from '@/api/types'
import type {
  RepoFileContentResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { RepositoryCodeView } from './repository-code-view'
import { RepositoryContext } from './repository-context'
import { RepositoryLatestActivity } from './repository-latest-activity'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { displayRouteFilePath } from '@/lib/route-file'
import type { CachedResource } from '@/lib/use-cached-resource'

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

  function selectResource(path: string) {
    workspaceTabs.open(displayRouteFilePath(path), false)
    onSelectFilePath(path)
  }

  return (
    <WorkbenchPane>
      <WorkbenchBar
        className="items-start border-b border-border"
        actions={(
          <>
            {content.value && repo.lifecycle_state === 'Ready' && (
              <RepoCloneDropdown
                cloneRemoteUrl={content.value.clone_remote_url}
                repo={repo}
              />
            )}
            <RepoPrimaryActionButton
              includeOpen={false}
              repo={repo}
              requireOwner
              variant="default"
            />
          </>
        )}
        summary={(
          <RepositoryContext
            content={content.value}
            contentLoading={content.status === 'loading'}
            onSelectFilePath={selectResource}
            repo={repo}
          />
        )}
        title="Code"
      />
      <RepositoryLatestActivity params={params} repo={repo} />
      <RepositoryCodeView
        content={content}
        file={file}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedPath={selectedPath}
        workspaceTabs={workspaceTabs}
      />
    </WorkbenchPane>
  )
}
