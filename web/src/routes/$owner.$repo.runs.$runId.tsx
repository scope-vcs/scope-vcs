import {
  cancelRepoRunForRequest,
  loadRepoRunStepLogsForRequest,
  parseRunActionInput,
  parseRunStepLogsInput,
  retryRepoRunForRequest,
} from '@/api/runs'
import { auth } from '@clerk/tanstack-react-start/server'
import type { RepoLiveState } from '@/api/types'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import type { RunActionInput, RunStepLogsInput } from '@/api/types'
import type {
  GitHubWorkflowRunDetailResponse,
  RepositoryRunDetailResponse,
} from '@/api/types.generated'
import { GitHubWorkflowRunDetailPage } from '@/features/runs/github-workflow-run-detail'
import { isGitHubRunId } from '@/features/runs/github-workflow-run-detail-model'
import {
  RepositoryRunDetailPage,
  RunDetailPageError,
} from '@/features/runs/repository-run-detail-page'
import { RunDetailPagePending } from '@/features/runs/run-detail-pending'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'
import {
  loadRepoGitHubWorkflowJobLog,
  loadRepoGitHubWorkflowRun,
  loadRepoRunDetail,
} from '@/routes/-run-history-actions'

const loadRepoRunStepLogs = createServerFn({ method: 'GET' })
  .validator(parseRunStepLogsInput)
  .handler(({ data }) => loadRepoRunStepLogsForRequest(data))

const cancelRepoRun = createServerFn({ method: 'POST' })
  .validator(parseRunActionInput)
  .handler(({ data }) => cancelRepoRunForRequest(data))

const retryRepoRun = createServerFn({ method: 'POST' })
  .validator(parseRunActionInput)
  .handler(({ data }) => retryRepoRunForRequest(data))

export const Route = createFileRoute('/$owner/$repo/runs/$runId')({
  loader: async ({ params, parentMatchPromise }) => {
    if (typeof window !== 'undefined') return null
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { userId } = await auth()
    const scope = repoResourceScope(live.repo, userId)
    return isGitHubRunId(params.runId)
      ? { kind: 'github' as const, scope, detail: await loadRepoGitHubWorkflowRun({ data: runInput(params) }) }
      : { kind: 'native' as const, scope, detail: await loadRepoRunDetail({ data: runInput(params) }) }
  },
  errorComponent: RunDetailPageError,
  pendingComponent: RunDetailPagePending,
  component: RepositoryRunDetailRoute,
})

function RepositoryRunDetailRoute() {
  const loaded = Route.useLoaderData()
  const { owner, repo, runId } = Route.useParams()
  const input = useMemo(
    () => runInput({ owner, repo, runId }),
    [owner, repo, runId],
  )
  const initialScope = loaded?.scope ?? null
  return isGitHubRunId(runId) ? (
    <GitHubRunDetailRoute
      initialDetail={loaded?.kind === 'github' ? loaded.detail : null}
      initialScope={initialScope}
      input={input}
      key={input.run_id}
    />
  ) : (
    <NativeRunDetailRoute
      initialDetail={loaded?.kind === 'native' ? loaded.detail : null}
      initialScope={initialScope}
      input={input}
      key={input.run_id}
    />
  )
}

function GitHubRunDetailRoute({
  initialDetail,
  initialScope,
  input,
}: {
  initialDetail: GitHubWorkflowRunDetailResponse | null
  initialScope: string | null
  input: RunActionInput
}) {
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
      initialDetail={initialDetail}
      initialScope={initialScope}
      loadDetail={loadDetail}
      loadLog={loadLog}
      params={input}
    />
  )
}

function NativeRunDetailRoute({
  initialDetail,
  initialScope,
  input,
}: {
  initialDetail: RepositoryRunDetailResponse | null
  initialScope: string | null
  input: RunActionInput
}) {
  const loadDetail = useCallback(
    (signal?: AbortSignal) => loadRepoRunDetail({ data: input, signal }),
    [input],
  )
  const loadLogs = useCallback(
    (data: RunStepLogsInput, signal?: AbortSignal) =>
      loadRepoRunStepLogs({ data, signal }),
    [],
  )
  const cancelRun = useCallback(
    () => cancelRepoRun({ data: input }).then(() => undefined),
    [input],
  )
  const retryRun = useCallback(
    () => retryRepoRun({ data: input }).then(() => undefined),
    [input],
  )

  return (
    <RepositoryRunDetailPage
      cancelRun={cancelRun}
      initialDetail={initialDetail}
      initialScope={initialScope}
      loadDetail={loadDetail}
      loadLogs={loadLogs}
      params={input}
      retryRun={retryRun}
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
