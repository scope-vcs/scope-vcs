import type { RepoParams, RepoRunHistoryInput } from '@/api/types'
import { RepositoryRunsPage } from '@/features/runs/repository-runs-page'
import {
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
