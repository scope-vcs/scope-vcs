import type { RepoContent, RepoParams } from '@/api/types'
import type { RepoLanguageResponse, RepoSummaryResponse } from '@/api/types.generated'
import { PanelState } from '@/components/empty-state'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { Button } from '@/components/ui/button'
import type { CachedResource } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { Code, FileQuestion, TriangleAlert } from 'lucide-react'
import { CloneButton, RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoViews } from './repo-layout-context'
import { hasRepositoryAbout, RepositoryAbout } from './repository-about'
import { ReadmeSkeleton } from './repository-code-skeletons'
import { RepositoryLanguageBar } from './repository-language-bar'
import { RepositoryFileContent } from './repository-file-content'
import type { RepoFile } from './repo-file-highlight'
import { RepoViewingAsPicker, useViewingAs } from './use-viewing-as'
import { ViewingAsStatus } from './viewing-as-picker'
import { Entrance } from '@/components/entrance'

const MAIN = { kind: 'main' } as const

export function RepoHomePage({
  content,
  file,
  languages,
  params,
  repo,
  selectedPath,
}: {
  content: CachedResource<RepoContent>
  file: CachedResource<RepoFile>
  languages: readonly RepoLanguageResponse[] | null
  params: RepoParams
  repo: RepoSummaryResponse
  selectedPath: string | null
}) {
  const { options, view } = useViewingAs()
  const views = useRepoViews()
  const ready = repo.lifecycle_state === 'Ready'

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            {options.length > 1 ? <RepoViewingAsPicker /> : <ViewingAsStatus name={views.name(view)} />}
            {ready && (content.value ? (
              <RepoCloneDropdown
                cloneRemoteUrl={content.value.clone_remote_url}
                repo={repo}
                view={view}
                viewName={views.name(view)}
              />
            ) : content.status === 'loading' && <CloneButton disabled />)}
            <RepoPrimaryActionButton
              repo={repo}
              requireOwner
              variant="default"
            />
            {ready ? (
              <Button asChild size="sm">
                <Link params={params} search={{}} to="/$owner/$repo/code">
                  <Code />
                  Browse code
                </Link>
              </Button>
            ) : (
              <Button disabled size="sm" type="button">
                <Code />
                Browse code
              </Button>
            )}
          </>
        )}
        summary={hasRepositoryAbout(repo) ? <RepositoryAbout repo={repo} /> : undefined}
        title="Home"
        titleHidden
      />
      <RepositoryLanguageBar languages={languages} />
      <section aria-label="Repository README">
        <RepositoryReadme content={content} file={file} params={params} ready={ready} selectedPath={selectedPath} />
      </section>
    </WorkbenchPane>
  )
}

function RepositoryReadme({
  content,
  file,
  params,
  ready,
  selectedPath,
}: {
  content: CachedResource<RepoContent>
  file: CachedResource<RepoFile>
  params: RepoParams
  ready: boolean
  selectedPath: string | null
}) {
  const files = content.value?.files
  if (!ready || files?.length === 0) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>Run scope push --main from the CLI to add files to this repository.</span>
      </PanelState>
    )
  }
  if (files && !selectedPath) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>No README yet. Add README.md or README.html to main and it shows here.</span>
      </PanelState>
    )
  }
  const error = content.error ?? file.error
  if (error && !file.value) {
    return (
      <PanelState role="alert" tone="error">
        <TriangleAlert className="size-5" />
        <span>{error}</span>
        <Button onClick={content.error ? content.retry : file.retry} size="sm" type="button" variant="secondary">
          Retry
        </Button>
      </PanelState>
    )
  }
  if (!file.value) {
    return (
      <PendingSurface className="min-h-[220px]" label="Loading repository README" onRetry={file.retry}>
        <ReadmeSkeleton />
      </PendingSurface>
    )
  }
  return (
    <Entrance key={file.value.oid}>
      <RepositoryFileContent file={file.value} htmlMode="preview" params={params} version={MAIN} />
    </Entrance>
  )
}
