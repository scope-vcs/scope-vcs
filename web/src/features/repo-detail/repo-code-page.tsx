import type { RepoContent, RepoParams } from '@/api/types'
import type {
  RepoFileContentResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { Button } from '@/components/ui/button'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { useRepositorySearch } from '@/features/repo-search/repository-search-context'
import type { CachedResource } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, ChevronDown, GitBranch, GitPullRequest, History } from 'lucide-react'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoViews } from './repo-layout-context'
import { RepositoryCodeView } from './repository-code-view'
import { RepositoryLatestActivity } from './repository-latest-activity'
import { RepositoryRequestStrip, RepositoryUpdateStrip } from './repository-version-strip'
import type { RepositoryVersion } from './repository-version'
import { RepoViewingAsPicker, useViewingAs } from './use-viewing-as'
import { ViewingAsStatus } from './viewing-as-picker'

export function RepoCodePage({
  content,
  file,
  onSelectFilePath,
  params,
  repo,
  selectedPath,
  version,
}: {
  content: CachedResource<RepoContent>
  file: CachedResource<RepoFileContentResponse>
  onSelectFilePath: (path: string) => void
  params: RepoParams
  repo: RepoSummaryResponse
  selectedPath: string | null
  version: RepositoryVersion
}) {
  const workspaceTabs = useWorkspaceTabs({ activeId: selectedPath })
  const { options, view } = useViewingAs()
  const views = useRepoViews()

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            {options.length > 1 ? <RepoViewingAsPicker /> : <ViewingAsStatus name={views.name(view)} />}
            {repo.lifecycle_state === 'Ready' && (content.value ? (
              <RepoCloneDropdown
                cloneRemoteUrl={content.value.clone_remote_url}
                repo={repo}
                view={view}
                viewName={views.name(view)}
              />
            ) : content.status === 'loading' && <BlockSkeleton className="h-8 w-24" />)}
          </>
        )}
        summary={(
          <div className="flex min-w-0 items-center gap-3">
            <Link
              className="flex shrink-0 items-center gap-1 rounded text-[13px] hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
              params={params}
              search={{}}
              to="/$owner/$repo"
            >
              <ArrowLeft aria-hidden="true" className="size-3.5" />
              Home
            </Link>
            <VersionButton version={version} />
          </div>
        )}
        title="Code"
      />
      {version.kind === 'main' && <RepositoryLatestActivity params={params} repo={repo} />}
      {version.kind === 'request' && <RepositoryRequestStrip params={params} repo={repo} requestId={version.id} />}
      {version.kind === 'update' && <RepositoryUpdateStrip entryId={version.id} params={params} repo={repo} selectedPath={selectedPath} />}
      <RepositoryCodeView
        content={content}
        file={file}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedPath={selectedPath}
        version={version}
        workspaceTabs={workspaceTabs}
      />
    </WorkbenchPane>
  )
}

function VersionButton({ version }: { version: RepositoryVersion }) {
  const search = useRepositorySearch()
  const Icon = version.kind === 'main' ? GitBranch : version.kind === 'request' ? GitPullRequest : History
  return (
    <Button
      aria-label="Change version"
      className="min-w-0"
      onClick={() => search.open()}
      size="sm"
      type="button"
      variant="secondary"
    >
      <Icon />
      <span className="truncate">
        {version.kind === 'main' ? 'main' : version.kind === 'request' ? 'Request' : 'Past update'}
      </span>
      <ChevronDown />
    </Button>
  )
}
