import type {
  RepoContent,
  RepoFileContent,
  RepoParams,
  RepoSummary,
} from '@/api/types'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { RepositoryCodeView } from './repository-code-view'
import { RepositoryContext } from './repository-context'
import { RepositoryLatestActivity } from './repository-latest-activity'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { displayRouteFilePath } from '@/lib/route-file'

export function RepoDetailPage({
  content,
  contentError,
  contentLoading,
  contentRetry,
  onSelectFilePath,
  params,
  repo,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
}: {
  content: RepoContent | null
  contentError: string | null
  contentLoading: boolean
  contentRetry: () => void
  onSelectFilePath: (path: string) => void
  params: RepoParams
  repo: RepoSummary
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
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
            {content && repo.lifecycle_state === 'Ready' && (
              <RepoCloneDropdown
                cloneRemoteUrl={content.clone_remote_url}
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
            content={content}
            contentLoading={contentLoading}
            onSelectFilePath={selectResource}
            repo={repo}
          />
        )}
        title="Code"
      />
      <RepositoryLatestActivity params={params} repo={repo} />
      <RepositoryCodeView
        content={content}
        contentError={contentError}
        contentRetry={contentRetry}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedFile={selectedFile}
        selectedFileError={selectedFileError}
        selectedFileIdentity={selectedFileIdentity}
        selectedFileLoading={selectedFileLoading}
        selectedFileRetry={selectedFileRetry}
        selectedPath={selectedPath}
        workspaceTabs={workspaceTabs}
      />
    </WorkbenchPane>
  )
}
