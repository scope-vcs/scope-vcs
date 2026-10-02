import type { RepoParams, RepoRunHistoryInput } from '@/api/types'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
import { RepositoryRunsPage } from '@/features/runs/repository-runs-page'
import {
  loadRepoGitHubWorkflowRuns,
  loadRepoRunHistory,
  loadRepoRunPage,
  loadRepoRunWorkflows,
} from '@/routes/-run-history-actions'
import { useCallback } from 'react'

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
    (data: RepoParams, signal: AbortSignal) => loadRepoGitHubWorkflowRuns({ data, signal }),
    [],
  )

  // GitHub keeps its own workflow filters, so every Runs route lists all runs.
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
      initialResources={initialResources}
      key={`${params.owner}/${params.repo}/${workflow ?? 'all'}/${initialResources ? 'member' : 'denied'}`}
      loadHistory={loadHistory}
      loadWorkflows={loadWorkflows}
      params={params}
      workflow={workflow}
    />
  )
}
