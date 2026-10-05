import type { RunActionInput, RunStepLogsInput } from '@/api/types'
import type {
  RepositoryRunDetailResponse,
  RepositoryRunStepLogPageResponse,
} from '@/api/types.generated'
import { WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { useRepositoryRunDetailController } from './repository-run-detail-controller'
import { RunDetailHeader } from './run-detail-header'
import { RunDetailJobs } from './run-detail-jobs'
import { useAuth } from '@clerk/tanstack-react-start'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { loadRunDetailSnapshot, runDetailResource, type RunDetailSnapshot } from './run-detail-resource'
import { useRunResource } from './run-resource'
import { RunDetailPagePending } from './run-detail-pending'
import { useCallback, useMemo } from 'react'
import { runLogCacheKey } from './run-log-cache'

type RunDetailPageProps = {
  cancelRun: () => Promise<void>
  initialDetail: RepositoryRunDetailResponse | null
  initialScope: string | null
  loadDetail: (signal?: AbortSignal) => Promise<RepositoryRunDetailResponse>
  loadLogs: (
    input: RunStepLogsInput,
    signal?: AbortSignal,
  ) => Promise<RepositoryRunStepLogPageResponse>
  params: RunActionInput
  retryRun: () => Promise<void>
}

export function RepositoryRunDetailPage(props: RunDetailPageProps) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const cacheKey = isLoaded
    ? runLogCacheKey(repoResourceScope(repo, userId ?? null), props.params.run_id)
    : null
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  return <RunDetailResourceView key={cacheKey ?? 'auth-pending'} cacheKey={cacheKey} scope={scope} {...props} />
}

function RunDetailResourceView({ cacheKey, scope, ...props }: RunDetailPageProps & { cacheKey: string | null; scope: string | null }) {
  const initialValue = useMemo<RunDetailSnapshot | null>(() => props.initialDetail && props.initialScope === scope ? {
    detail: props.initialDetail, generation: 0, updatedAt: Date.now(),
  } : null, [props.initialDetail, props.initialScope, scope])
  const { loadDetail } = props
  const load = useCallback(async (signal: AbortSignal): Promise<RunDetailSnapshot> => {
    if (!cacheKey) throw new Error('Run detail scope is unavailable.')
    return loadRunDetailSnapshot(cacheKey, loadDetail, signal)
  }, [cacheKey, loadDetail])
  const resource = useRunResource({ identity: cacheKey, initialValue, load, resource: runDetailResource })
  if (!resource.value) return resource.error ? <RunDetailPageError error={resource.error} /> : <RunDetailPagePending />
  return <RunDetailView {...props} cacheKey={cacheKey} initialDetail={resource.value.detail} />
}

function RunDetailView({
  cacheKey,
  cancelRun,
  initialDetail,
  loadDetail,
  loadLogs,
  params,
  retryRun,
}: Omit<RunDetailPageProps, 'initialDetail'> & { cacheKey: string | null; initialDetail: RepositoryRunDetailResponse }) {
  const {
    actionError,
    attemptOverrides,
    detail,
    metadataError,
    pendingAction,
    performAction,
    refreshDetail,
    selectAttempt,
    selectedJobKey,
    selection,
    showGraph,
    showJob,
    stepLogs,
    toggleGraph,
    toggleStep,
  } = useRepositoryRunDetailController({
    cacheKey,
    initialDetail,
    loadDetail,
    loadLogs,
    params,
  })

  return (
    <WorkbenchPane className="flex flex-col lg:h-[calc(100dvh-var(--app-topbar))]">
      <RunDetailHeader
        detail={detail}
        metadataError={metadataError}
        onCancel={() => void performAction('cancel', cancelRun)}
        onRefresh={() => void refreshDetail()}
        onRetry={() => void performAction('retry', retryRun)}
        params={params}
        pendingAction={pendingAction}
      />
      {actionError ? (
        <div className="px-5 pb-5 sm:px-6 lg:px-8">
          <PageErrorAlert title="Run action failed">
            {actionError}
          </PageErrorAlert>
        </div>
      ) : null}
      <RunDetailJobs
        attemptOverrides={attemptOverrides}
        jobs={detail.jobs}
        onSelectAttempt={selectAttempt}
        onSelectJob={showJob}
        onSelectStep={toggleStep}
        onToggleGraph={toggleGraph}
        selectedJobKey={selectedJobKey}
        selection={selection}
        showGraph={showGraph}
        stepLogs={stepLogs}
      />
    </WorkbenchPane>
  )
}

export function RunDetailPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected run detail error"
        title="Run unavailable"
      />
    </WorkbenchPane>
  )
}
