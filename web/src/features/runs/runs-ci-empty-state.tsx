import type { RepoParams } from '@/api/types'
import type { GitHubAuthorizeResponse } from '@/api/types.generated'
import { EmptyState } from '@/components/empty-state'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { resourceErrorMessage, useCachedResource } from '@/lib/use-cached-resource'
import { useAuth } from '@clerk/tanstack-react-start'
import { LoaderCircle, Plug, TerminalSquare } from 'lucide-react'
import { useCallback, useState } from 'react'
import { openGitHubAuthorization } from '../github/github-authorization'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { type RepoSettingsData, repoSettingsResource } from '../repo-detail/repo-settings-resource'
import { NoRunsEmptyState } from './run-history-list'
import { RUNS_COME_FROM_GITHUB, runsCiOffer } from './runs-ci-offer-model'

export type RunsGitHubActions = {
  configured: boolean
  loadSettings: (params: RepoParams, signal: AbortSignal) => Promise<RepoSettingsData>
  startAuthorization: (params: RepoParams) => Promise<GitHubAuthorizeResponse>
}

export function RunsCiEmptyState({
  github,
  hasWorkflows,
  params,
}: {
  github: RunsGitHubActions
  hasWorkflows: boolean
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const maintainer = repo.access.actor !== 'Public'
  const asksGitHub = github.configured && !hasWorkflows
  const scope = isLoaded && maintainer && asksGitHub ? repoResourceScope(repo, userId ?? null) : null
  const { loadSettings, startAuthorization } = github
  const { owner, repo: repoName } = params
  const settings = useCachedResource({
    fallbackError: 'Repository settings could not be loaded.',
    identity: scope,
    load: useCallback(
      (signal: AbortSignal) => loadSettings({ owner, repo: repoName }, signal),
      [loadSettings, owner, repoName],
    ),
    resource: repoSettingsResource,
  })
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const offer = runsCiOffer({
    configured: github.configured,
    github: settings.value?.github ?? null,
    hasWorkflows,
    maintainer,
  })
  if (offer.kind === 'none') return <NoRunsEmptyState />

  async function connect() {
    setError(null)
    setPending(true)
    try {
      await openGitHubAuthorization(() => startAuthorization({ owner, repo: repoName }))
    } catch (cause) {
      setPending(false)
      setError(resourceErrorMessage(cause, 'Try again.'))
    }
  }

  return (
    <div>
      <EmptyState
        action={offer.canConnect ? (
          <Button disabled={pending} onClick={() => void connect()} size="sm" type="button">
            {pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <Plug className="size-3.5" />}
            <span>Connect GitHub</span>
          </Button>
        ) : undefined}
        description={RUNS_COME_FROM_GITHUB}
        icon={<TerminalSquare />}
        title="No runs yet"
      />
      {error && <PageErrorAlert title="GitHub could not be opened">{error}</PageErrorAlert>}
    </div>
  )
}
