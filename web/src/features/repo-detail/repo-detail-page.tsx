import type { RepoContent, RepoParams } from '@/api/types'
import type {
  RepoFileContentResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { ExternalLink, Eye } from 'lucide-react'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { RepositoryCodeView } from './repository-code-view'
import { RepositoryLatestActivity } from './repository-latest-activity'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
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

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
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
        summary={<RepositoryAbout repo={repo} />}
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

function RepositoryAbout({ repo }: { repo: RepoSummaryResponse }) {
  if (!repo.description && !repo.website_url) return null
  return (
    <p className="flex min-w-0 items-center gap-x-3 text-xs">
      {repo.description && <span className="min-w-0 truncate" title={repo.description}>{repo.description}</span>}
      {repo.website_url && (
        <a
          className="inline-flex min-w-0 max-w-1/2 shrink-0 items-center gap-1 rounded hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          href={repo.website_url}
          rel="noopener noreferrer"
          target="_blank"
          title={repo.website_url}
        >
          <span className="truncate">{websiteLabel(repo.website_url)}</span>
          <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
        </a>
      )}
    </p>
  )
}

function websiteLabel(url: string) {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '')
}
