import type { RepoLanguageResponse, RepoSummaryResponse } from '@/api/types.generated'
import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { loadRepoLanguages } from '@/routes/-repo-code-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback } from 'react'
import { createRepoResource, repoResourceKey } from './repo-resource'
import { repoResourceScope } from './repo-resource-scope'
import { useViewingAs } from './use-viewing-as'

const repoLanguagesResource = createRepoResource<RepoLanguageResponse[]>({
  invalidatedBy: ['content'],
  maxEntries: 16,
})

export function useRepoLanguages({
  params,
  repo,
}: {
  params: { owner: string; repo: string }
  repo: RepoSummaryResponse
}) {
  const { userId, isLoaded } = useAuth()
  const { view } = useViewingAs()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const load = useCallback((signal: AbortSignal) => loadRepoLanguages({
    data: { owner, repo: repoName, view }, signal,
  }), [owner, repoName, view])
  const languages = useCachedResource({
    enabled: repo.lifecycle_state === 'Ready',
    fallbackError: 'Repository languages are unavailable.',
    identity: scope ? repoResourceKey(scope, view) : null,
    load,
    resource: repoLanguagesResource,
    version: String(repo.content_version),
  })
  useRetryOnReconnect(languages)
  return languages
}
