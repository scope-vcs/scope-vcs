import type { RepoParams, RepoRunHistoryInput } from '@/api/types'
import type {
  RepositoryRunHistoryPageResponse,
  RepositoryRunWorkflowListResponse,
} from '@/api/types.generated'
import { resourceErrorMessage, useCachedResource } from '@/lib/use-cached-resource'
import { PageContent, WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { useCallback, useMemo, useState, useSyncExternalStore } from 'react'
import { RunHistoryList } from './run-history-list'
import { RunsCiEmptyState, type RunsGitHubActions } from './runs-ci-empty-state'
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
import { runWorkflowsResource } from './run-workflows-resource'

const HISTORY_CHANGES = ['Created', 'StatusChanged'] as const

type RunPageResources = {
  history: RepositoryRunHistoryPageResponse
  workflows: RepositoryRunWorkflowListResponse
  workflowsError: string | null
}

type RepositoryRunsPageProps = {
  /** How an empty page offers to connect GitHub. */
  github: RunsGitHubActions | null
  initialResources: RunPageResources | null
  loadHistory: (
    input: RepoRunHistoryInput,
    signal?: AbortSignal,
  ) => Promise<RepositoryRunHistoryPageResponse | null>
  loadWorkflows: (
    input: RepoParams,
    signal?: AbortSignal,
  ) => Promise<RepositoryRunWorkflowListResponse | null>
  params: RepoParams
  workflow?: string
}

export function RepositoryRunsPage(props: RepositoryRunsPageProps) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded && props.initialResources
    ? repoResourceScope(repo, userId ?? null)
    : null
  const cacheKey = scope ? runHistoryCacheKey(scope, props.workflow) : null
  return <RepositoryRunsPageContent github={props.github} initialResources={props.initialResources} loadHistory={props.loadHistory} loadWorkflows={props.loadWorkflows} params={props.params} workflow={props.workflow} key={cacheKey ?? 'unavailable'} cacheKey={cacheKey} scope={scope} />
}

function RepositoryRunsPageContent({
  cacheKey,
  github,
  initialResources,
  loadHistory,
  loadWorkflows,
  params,
  scope,
  workflow,
}: RepositoryRunsPageProps & { cacheKey: string | null; scope: string | null }) {
  const [key] = useState(() => cacheKey ?? crypto.randomUUID())
  useState(() => initializeRunHistory(key, initialResources?.history ?? null))
  const snapshot = useSyncExternalStore(
    useCallback((listener) => runHistoryResource.subscribe(key, listener), [key]),
    useCallback(() => runHistoryResource.getSnapshot(key), [key]),
    runHistoryResource.getServerSnapshot,
  )
  const history = snapshot.value ? snapshot.value.history : initialResources?.history ?? null
  const refreshError = snapshot.error === null ? null : resourceErrorMessage(snapshot.error, 'Run operation failed.')
  const loadingMore = snapshot.pending && snapshot.version === 'more'
  const [statusFilter, setStatusFilter] = useState<RunStatusFilter>('any')
  const { owner, repo } = params
  const input = useMemo(() => ({ owner, repo, workflow }), [owner, repo, workflow])
  const workflowResource = useCachedResource({
    fallbackError: 'Workflow catalog unavailable.',
    identity: scope,
    initialValue: initialResources?.workflowsError ? null : initialResources?.workflows,
    load: useCallback(async (signal: AbortSignal) => {
      const workflows = await loadWorkflows({ owner, repo }, signal)
      if (!workflows) throw new Error('Workflow catalog unavailable.')
      return workflows
    }, [loadWorkflows, owner, repo]),
    resource: runWorkflowsResource,
  })
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

  const workflows = workflowResource.value ?? initialResources.workflows
  const workflowsError = workflowResource.value
    ? null
    : workflowResource.error ?? initialResources.workflowsError
  const nativeRunsAvailable = workflows.native_runs_available
  const selectedWorkflow = workflow
    ? workflows.workflows.find((item) => item.key === workflow)
    : undefined

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <RunsFilterBar
            onStatusFilterChange={setStatusFilter}
            params={params}
            selectedWorkflow={workflow}
            showWorkflowFilter={nativeRunsAvailable}
            statusFilter={statusFilter}
            workflows={workflows.workflows}
          />
        )}
        title="Runs"
      />
      <div className="min-w-0 border-t border-border">
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
          {nativeRunsAvailable ? null : (
            <p className="pt-5 text-sm text-muted-foreground">
              Scope runs are not available for this repository.
            </p>
          )}
          {workflowsError ? (
            <div className="pt-5">
              <PageErrorAlert title="Workflow filter unavailable">
                <div>
                  <p>Run history is still available.</p>
                  <p className="mt-1 text-xs">{workflowsError}</p>
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
              empty={github && !workflow ? (
                <RunsCiEmptyState
                  github={github}
                  // Native workflows count only where Scope may run them.
                  hasWorkflows={nativeRunsAvailable && workflows.workflows.length > 0}
                  params={params}
                />
              ) : undefined}
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
