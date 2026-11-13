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
import { RUNS_COME_FROM_GITHUB, runsCiOffer } from './runs-ci-offer-model'

type RunsGitHubActions = {
  configured: boolean
  loadSettings: (params: RepoParams, signal: AbortSignal) => Promise<RepoSettingsData>
  startAuthorization: (params: RepoParams) => Promise<GitHubAuthorizeResponse>
}

export function RunsCiEmptyState({
  github,
  params,
}: {
  github: RunsGitHubActions
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const maintainer = repo.access.actor !== 'Public'
  const scope = isLoaded && maintainer && github.configured ? repoResourceScope(repo, userId ?? null) : null
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
    maintainer,
  })
  if (offer.kind === 'none') return <EmptyState icon={<TerminalSquare />} title="No runs yet" />

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
