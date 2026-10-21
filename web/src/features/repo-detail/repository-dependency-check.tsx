import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { displayRouteFilePath } from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadRepositoryDependencies } from '@/routes/-repo-dependency-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useNavigate } from '@tanstack/react-router'
import { Network } from 'lucide-react'
import { useCallback } from 'react'
import { useRepoViews } from './repo-layout-context'
import { DependencyCheckDetailsBody } from './repository-dependency-check-view'
import { dependencyCheckPresentation, type DependencyCheckPresentation } from './repository-dependency-model'
import {
  repositoryDependencyIdentity,
  repositoryDependencyResource,
} from './repository-dependency-resource'
import { viewingAsSearch } from './viewing-as'

export function RepositoryDependencySection({ params, repo }: { params: RepoParams; repo: RepoSummaryResponse }) {
  const presentation = useRepositoryDependencyCheck({ params, repo })
  const views = useRepoViews()
  const navigate = useNavigate()
  const fullView = views.full
  if (presentation.kind === 'hidden' || fullView === null) return null

  const openFile = (path: string) => void navigate({
    params,
    search: { file: displayRouteFilePath(path), ...viewingAsSearch(fullView, repo.access.view) },
    to: '/$owner/$repo',
  })

  return (
    <SectionRows>
      <SectionRow
        description="Finds public JS/TS files that import private files."
        icon={<Network className="size-4" />}
        id="dependency-check"
        title="Dependency check"
      >
        <p className="flex flex-wrap items-baseline gap-x-3 text-sm text-warning-strong">
          <span>{presentation.label}</span>
          {presentation.kind === 'leak' && <span className="text-xs text-muted-foreground">{presentation.meta}</span>}
        </p>
        {presentation.details && (
          <div className="mt-3 text-xs">
            <DependencyCheckDetailsBody details={presentation.details} onSelectFilePath={openFile} views={views} />
          </div>
        )}
      </SectionRow>
    </SectionRows>
  )
}

function useRepositoryDependencyCheck({
  params,
  repo,
}: {
  params: RepoParams
  repo: RepoSummaryResponse
}): DependencyCheckPresentation {
  const { isLoaded, userId } = useAuth()
  const views = useRepoViews()
  const { owner, repo: repoName } = params
  const readsFullView = views.full !== null && views.mayRead(repo.access.view, views.full)
  const identity = isLoaded && repo.lifecycle_state === 'Ready' && readsFullView
    ? repositoryDependencyIdentity(repo, userId ?? null)
    : null
  const load = useCallback((signal: AbortSignal) => loadRepositoryDependencies({
    data: { owner, repo: repoName },
    signal,
  }), [owner, repoName])
  const resource = useCachedResource({
    fallbackError: 'Dependency check unavailable.',
    identity,
    load,
    resource: repositoryDependencyResource,
    version: String(repo.content_version),
  })

  if (!identity) return { kind: 'hidden' }
  return dependencyCheckPresentation({
    refreshError: resource.error,
    refreshing: resource.refreshing,
    response: resource.value,
  })
}
