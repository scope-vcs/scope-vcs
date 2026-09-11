import type { RepoParams, RepoRunHistoryInput } from '@/api/types'
import type {
  RepositoryRunHistoryPageResponse,
  RepositoryRunWorkflowListResponse,
} from '@/api/types.generated'
import { PageContent, WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { useCallback, useMemo, useState, useSyncExternalStore } from 'react'
import { RunHistoryList } from './run-history-list'
import { useRunLiveRefresh } from './run-live-refresh'
import { RunsFilterBar } from './runs-filter-bar'
import {
  type RunStatusFilter,
  runMatchesStatusFilter,
} from './runs-filter-model'

import { useAuth } from '@clerk/tanstack-react-start'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { initializeRunHistory, loadMoreRunHistory, refreshRunHistory, runHistoryCacheKey, runHistoryResource } from './run-history-cache'

const HISTORY_CHANGES = ['Created', 'StatusChanged'] as const

type RunPageResources = {
  history: RepositoryRunHistoryPageResponse
  workflows: RepositoryRunWorkflowListResponse
  workflowsError: string | null
}

type RepositoryRunsPageProps = {
  initialResources: RunPageResources | null
  loadHistory: (
    input: RepoRunHistoryInput,
    signal?: AbortSignal,
  ) => Promise<RepositoryRunHistoryPageResponse | null>
  params: RepoParams
  workflow?: string
}

export function RepositoryRunsPage(props: RepositoryRunsPageProps) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const cacheKey = isLoaded && props.initialResources
    ? runHistoryCacheKey(repoResourceScope(repo, userId ?? null), props.workflow)
    : null
  return <RepositoryRunsPageContent initialResources={props.initialResources} loadHistory={props.loadHistory} params={props.params} workflow={props.workflow} key={cacheKey ?? 'unavailable'} cacheKey={cacheKey} />
}

function RepositoryRunsPageContent({
  cacheKey,
  initialResources,
  loadHistory,
  params,
  workflow,
}: RepositoryRunsPageProps & { cacheKey: string | null }) {
  const [key] = useState(() => cacheKey ?? crypto.randomUUID())
  useState(() => initializeRunHistory(key, initialResources?.history ?? null))
  const snapshot = useSyncExternalStore(
    useCallback((listener) => runHistoryResource.subscribe(key, listener), [key]),
    useCallback(() => runHistoryResource.getSnapshot(key), [key]),
    runHistoryResource.getServerSnapshot,
  )
  const history = snapshot.value ? snapshot.value.history : initialResources?.history ?? null
  const refreshError = snapshot.error === null ? null : snapshot.error instanceof Error ? snapshot.error.message : 'Run operation failed.'
  const loadingMore = snapshot.pending && snapshot.version === 'more'
  const [statusFilter, setStatusFilter] = useState<RunStatusFilter>('any')
  const { owner, repo } = params
  const input = useMemo(() => ({ owner, repo, workflow }), [owner, repo, workflow])
  const refreshRuns = useRunLiveRefresh({
    acceptedChanges: HISTORY_CHANGES,
    mutable: history !== null,
    refresh: useCallback(async () => {
      await refreshRunHistory({ key, input, loadHistory })
    }, [key, input, loadHistory]),
  })
  const loadMore = () => loadMoreRunHistory({ key, input, loadHistory })
  const filteredRuns = useMemo(() => history
    ? history.runs.filter((run) => runMatchesStatusFilter(run, statusFilter)) : [],
  [history, statusFilter])

  if (!initialResources || !history) {
    return (
      <PageContent>
        <h1 className="sr-only">Runs</h1>
        <PageErrorAlert title="Runs unavailable">
          Sign in as the owner or a repository member to view runs.
        </PageErrorAlert>
      </PageContent>
    )
  }

  const selectedWorkflow = workflow
    ? initialResources.workflows.workflows.find((item) => item.key === workflow)
    : undefined

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <RunsFilterBar
            onStatusFilterChange={setStatusFilter}
            params={params}
            selectedWorkflow={workflow}
            statusFilter={statusFilter}
            workflows={initialResources.workflows.workflows}
          />
        )}
        title="Runs"
      />
      <div className="min-w-0 border-t border-border">
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
          {initialResources.workflowsError ? (
            <div className="pt-5">
              <PageErrorAlert title="Workflow filter unavailable">
                <div>
                  <p>Run history is still available.</p>
                  <p className="mt-1 text-xs">{initialResources.workflowsError}</p>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          {refreshError ? (
            <div className="pt-5">
              <PageErrorAlert title="Runs could not refresh">
                <div className="flex flex-wrap items-center gap-3">
                  <span>{refreshError}</span>
                  <Button onClick={refreshRuns} size="sm" variant="secondary">
                    Retry now
                  </Button>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          <div className="pt-7">
            <RunHistoryList
              loadMore={() => void loadMore()}
              loadingMore={loadingMore}
              params={params}
              runs={filteredRuns}
              selectedWorkflowName={selectedWorkflow?.name}
              showLoadMore={history.next_cursor !== null}
              totalRunCount={history.runs.length}
            />
          </div>
        </main>
      </div>
    </WorkbenchPane>
  )
}
