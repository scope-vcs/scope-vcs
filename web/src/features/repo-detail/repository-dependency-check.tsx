import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadRepositoryDependencies } from '@/routes/-repo-dependency-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback } from 'react'
import { RepositoryDependencyCheckView } from './repository-dependency-check-view'
import { dependencyCheckPresentation } from './repository-dependency-model'
import {
  repositoryDependencyIdentity,
  repositoryDependencyResource,
} from './repository-dependency-resource'

export function RepositoryDependencyCheck({
  onSelectFilePath,
  params,
  repo,
}: {
  onSelectFilePath: (path: string) => void
  params: RepoParams
  repo: RepoSummaryResponse
}) {
  const { isLoaded, userId } = useAuth()
  const { owner, repo: repoName } = params
  const identity = isLoaded && repo.lifecycle_state === 'Ready'
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
    version: String(repo.change_version),
  })

  if (!identity) return null
  return (
    <RepositoryDependencyCheckView
      onSelectFilePath={onSelectFilePath}
      presentation={dependencyCheckPresentation({
        refreshError: resource.error,
        refreshing: resource.refreshing,
        response: resource.value,
      })}
    />
  )
}
