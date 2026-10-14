import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadRepositoryDependencies } from '@/routes/-repo-dependency-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback } from 'react'
import { useRepoViews } from './repo-layout-context'
import { dependencyCheckPresentation, type DependencyCheckPresentation } from './repository-dependency-model'
import {
  repositoryDependencyIdentity,
  repositoryDependencyResource,
} from './repository-dependency-resource'

export function useRepositoryDependencyCheck({
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
