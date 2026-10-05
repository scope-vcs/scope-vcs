import type { RepoGitHubWorkflowRunsInput, RepoParams, RepoRunHistoryInput } from '@/api/types'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
import { RepositoryRunsPage } from '@/features/runs/repository-runs-page'
import {
  loadRepoGitHubWorkflowRuns,
  loadRepoRunHistory,
  loadRepoRunPage,
  loadRepoRunWorkflows,
} from '@/routes/-run-history-actions'
import { loadRepoSettingsData, startRepoGitHubAuthorization } from '@/routes/-repo-settings-actions'
import { useCallback, useMemo } from 'react'

import { useAuth } from '@clerk/tanstack-react-start'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { loadRunPageSnapshot, runPageSnapshot, runHistoryCacheKey, runHistoryResource, type RetainedRunHistory, type RunPageHandoff } from './run-history-cache'
import { useRunResource } from './run-resource'
import { RunsPagePending } from './runs-page-pending'
import { RunsPageError } from './runs-page-error'

export function RepositoryRunsRoute(props: { initialResources: RunPageHandoff | null; params: RepoParams; workflow?: string }) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  return <RunRouteContent {...props} scope={scope} key={scope ? runHistoryCacheKey(scope, props.workflow) : 'auth-pending'} />
}

function RunRouteContent({
  initialResources,
  params,
  workflow,
  scope,
}: {
  initialResources: RunPageHandoff | null
  scope: string | null
  params: RepoParams
  workflow?: string
}) {
  const loadPage = useCallback((input: RepoRunHistoryInput, signal?: AbortSignal) => loadRepoRunPage({ data: input, signal }), [])
  const loadHistory = useCallback(
    (input: RepoRunHistoryInput, signal?: AbortSignal) =>
      loadRepoRunHistory({ data: input, signal }),
    [],
  )
  const loadWorkflows = useCallback(
    (input: RepoParams, signal?: AbortSignal) =>
      loadRepoRunWorkflows({ data: input, signal }),
    [],
  )
  const loadGitHubRuns = useCallback(
    (data: RepoGitHubWorkflowRunsInput, signal: AbortSignal) =>
      loadRepoGitHubWorkflowRuns({ data, signal }),
    [],
  )

  const identity = scope ? runHistoryCacheKey(scope, workflow) : null
  const initialValue = useMemo<RetainedRunHistory | null>(() => {
    if (!scope || initialResources?.scope !== scope) return null
    return runPageSnapshot(initialResources.resources)
  }, [scope, initialResources])
  const { owner, repo } = params
  const load = useCallback(async (signal: AbortSignal): Promise<RetainedRunHistory> => {
    if (!identity) throw new Error('Run history scope is unavailable.')
    return loadRunPageSnapshot({
      key: identity, input: { owner, repo, workflow }, loadHistory, signal,
      loadPage,
    })
  }, [identity, owner, repo, workflow, loadHistory, loadPage])
  const resource = useRunResource({ identity, initialValue, load, resource: runHistoryResource, refreshVersion: 'refresh' })
  const page = resource.value?.page
  const configured = page?.kind === 'native' && page.githubConfigured
  const github = useMemo(() => ({
    configured,
    loadSettings: loadRepoSettingsData,
    startAuthorization: startRepoGitHubAuthorization,
  }), [configured])

  if (!resource.value) return resource.error ? <RunsPageError error={resource.error} /> : <RunsPagePending />

  // GitHub's runs are filtered on the page itself, so every Runs route,
  // a workflow's route included, starts from all of them.
  if (page?.kind === 'github') {
    return (
      <GitHubWorkflowRunsPage
        initialRuns={page.github}
        key={`${params.owner}/${params.repo}`}
        loadRuns={loadGitHubRuns}
        params={params}
      />
    )
  }

  return (
    <RepositoryRunsPage
      github={github}
      initialResources={page?.kind === 'native' && resource.value.history ? page : null}
      key={`${params.owner}/${params.repo}/${workflow ?? 'all'}/${page ? 'member' : 'denied'}`}
      loadPage={loadPage}
      loadHistory={loadHistory}
      loadWorkflows={loadWorkflows}
      params={params}
      workflow={workflow}
    />
  )
}
