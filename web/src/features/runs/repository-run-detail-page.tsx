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
import { runLogCacheKey } from './run-log-cache'

type RunDetailPageProps = {
  cancelRun: () => Promise<void>
  initialDetail: RepositoryRunDetailResponse
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
  return <RunDetailView cancelRun={props.cancelRun} initialDetail={props.initialDetail} loadDetail={props.loadDetail} loadLogs={props.loadLogs} params={props.params} retryRun={props.retryRun} cacheKey={cacheKey} key={cacheKey ?? 'auth-pending'} />
}

function RunDetailView({
  cacheKey,
  cancelRun,
  initialDetail,
  loadDetail,
  loadLogs,
  params,
  retryRun,
}: RunDetailPageProps & { cacheKey: string | null }) {
  const {
    actionError,
    attemptOverrides,
    detail,
    metadataError,
    pendingAction,
    performAction,
    refreshDetail,
    refreshLogs,
    selectAttempt,
    selectedJobKey,
    selectedLogState,
    selection,
    showGraph,
    toggleGraph,
    toggleJob,
    toggleStep,
  } = useRepositoryRunDetailController({
    cacheKey,
    initialDetail,
    loadDetail,
    loadLogs,
    params,
  })

  return (
    <WorkbenchPane>
      <RunDetailHeader
        detail={detail}
        metadataError={metadataError}
        onCancel={() => void performAction('cancel', cancelRun)}
        onRefresh={() => void refreshDetail()}
        onRetry={() => void performAction('retry', retryRun)}
        params={params}
        pendingAction={pendingAction}
      />
      <main className="px-4 pb-14 sm:px-6 lg:px-8">
        {actionError ? (
          <div className="pt-5">
            <PageErrorAlert title="Run action failed">
              {actionError}
            </PageErrorAlert>
          </div>
        ) : null}
        <RunDetailJobs
          attemptOverrides={attemptOverrides}
          jobs={detail.jobs}
          onLogRetry={() => {
            if (selection) void refreshLogs(selection, 'retry')
          }}
          onLogEarlier={() => {
            if (selection) void refreshLogs(selection, 'earlier')
          }}
          onLogLatest={() => {
            if (selection) void refreshLogs(selection, 'latest')
          }}
          onSelectAttempt={selectAttempt}
          onSelectJob={toggleJob}
          onSelectStep={toggleStep}
          onToggleGraph={toggleGraph}
          selectedJobKey={selectedJobKey}
          selectedLogState={selectedLogState}
          selection={selection}
          showGraph={showGraph}
        />
      </main>
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
