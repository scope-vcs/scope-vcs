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

type RunPageResources = Awaited<ReturnType<typeof loadRepoRunPage>>

export function RepositoryRunsRoute({
  initialResources,
  params,
  workflow,
}: {
  initialResources: RunPageResources
  params: RepoParams
  workflow?: string
}) {
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

  const configured = initialResources?.kind === 'native' && initialResources.githubConfigured
  const github = useMemo(() => ({
    configured,
    loadSettings: loadRepoSettingsData,
    startAuthorization: startRepoGitHubAuthorization,
  }), [configured])

  if (initialResources?.kind === 'github') {
    return (
      <GitHubWorkflowRunsPage
        initialRuns={initialResources.github}
        key={`${params.owner}/${params.repo}`}
        loadRuns={loadGitHubRuns}
        params={params}
      />
    )
  }

  return (
    <RepositoryRunsPage
      github={github}
      initialResources={initialResources}
      key={`${params.owner}/${params.repo}/${workflow ?? 'all'}/${initialResources ? 'member' : 'denied'}`}
      loadHistory={loadHistory}
      loadWorkflows={loadWorkflows}
      params={params}
      workflow={workflow}
    />
  )
}
