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
import {
  RepositoryRunDetailPage,
  RunDetailPageError,
} from '@/features/runs/repository-run-detail-page'
import { RunDetailPagePending } from '@/features/runs/run-detail-pending'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'
import { loadRepoRunDetail } from '@/routes/-run-history-actions'

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
    return { scope: repoResourceScope(live.repo, userId), detail: await loadRepoRunDetail({ data: runInput(params) }) }
  },
  errorComponent: RunDetailPageError,
  pendingComponent: RunDetailPagePending,
  component: RepositoryRunDetailRoute,
})

function RepositoryRunDetailRoute() {
  const initialDetail = Route.useLoaderData()
  const { owner, repo, runId } = Route.useParams()
  const input = useMemo(
    () => runInput({ owner, repo, runId }),
    [owner, repo, runId],
  )
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
      initialDetail={initialDetail?.detail ?? null}
      initialScope={initialDetail?.scope ?? null}
      key={input.run_id}
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
