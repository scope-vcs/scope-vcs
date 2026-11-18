import type { RunActionInput } from '@/api/types'
import { GitHubWorkflowRunDetailPage, RunDetailPageError } from '@/features/runs/github-workflow-run-detail'
import { isGitHubRunId } from '@/features/runs/github-workflow-run-detail-model'
import { RunDetailPagePending } from '@/features/runs/run-detail-pending'
import { createFileRoute, notFound } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'
import {
  loadRepoGitHubWorkflowJobLog,
  loadRepoGitHubWorkflowRun,
} from '@/routes/-run-history-actions'

export const Route = createFileRoute('/$owner/$repo/runs/$runId')({
  loader: ({ params }) => {
    if (!isGitHubRunId(params.runId)) notFound({ throw: true })
  },
  errorComponent: RunDetailPageError,
  pendingComponent: RunDetailPagePending,
  component: RepositoryRunDetailRoute,
})

function RepositoryRunDetailRoute() {
  const { owner, repo, runId } = Route.useParams()
  const input = useMemo(
    () => runInput({ owner, repo, runId }),
    [owner, repo, runId],
  )
  const loadDetail = useCallback(
    (signal: AbortSignal) => loadRepoGitHubWorkflowRun({ data: input, signal }),
    [input],
  )
  const loadLog = useCallback(
    (jobId: string, signal: AbortSignal) =>
      loadRepoGitHubWorkflowJobLog({ data: { ...input, job_id: jobId }, signal }),
    [input],
  )
  return (
    <GitHubWorkflowRunDetailPage
      key={input.run_id}
      loadDetail={loadDetail}
      loadLog={loadLog}
      params={input}
    />
  )
}

function runInput(params: {
  owner: string
  repo: string
  runId: string
}): RunActionInput {
  return {
    owner: params.owner,
    repo: params.repo,
    run_id: params.runId,
  }
}
