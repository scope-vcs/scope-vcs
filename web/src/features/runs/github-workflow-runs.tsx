import type { RepoGitHubWorkflowRunsInput, RepoParams } from '@/api/types'
import type {
  GitHubWorkflowRunListResponse,
  GitHubWorkflowNamesResponse,
  GitHubWorkflowRunsResponse,
} from '@/api/types.generated'
import { EmptyState } from '@/components/empty-state'
import { PageErrorAlert } from '@/components/page-error-alert'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useCachedResource } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { ExternalLink, FlaskConical, LoaderCircle, TerminalSquare } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import {
  type GitHubWorkflowRunRow,
  githubWorkflowFilterOptions,
  githubWorkflowRunRow,
  reloadGitHubWorkflowRunPages,
} from './github-workflow-run-model'
import { seedGitHubWorkflowRunDetail } from './github-workflow-run-detail-resource'
import {
  githubWorkflowRunsIdentity,
  githubWorkflowRunsResource,
  loadMoreGitHubWorkflowRuns,
} from './github-workflow-runs-resource'
import { RunStatusIcon } from './run-status-icon'
import { githubWorkflowNamesResource } from './github-workflow-names-resource'
import { RUN_ROW_CLASS, RUN_ROW_TIMESTAMP_CLASS } from './run-row-layout'

const LINK_CLASS = 'underline-offset-2 hover:text-foreground hover:underline'
const SELECT_CLASS = 'h-8 max-w-44 rounded-md border border-input bg-secondary px-2 text-sm text-foreground shadow-[var(--shadow-card)] outline-none transition-colors focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring'

export function GitHubWorkflowRunsPage({
  initialRuns,
  initialNames,
  loadRuns,
  loadNames,
  params,
}: {
  initialRuns: GitHubWorkflowRunListResponse
  initialNames: GitHubWorkflowNamesResponse
  loadRuns: (input: RepoGitHubWorkflowRunsInput, signal: AbortSignal) => Promise<GitHubWorkflowRunsResponse>
  loadNames: (input: RepoParams, signal: AbortSignal) => Promise<GitHubWorkflowNamesResponse>
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const [workflow, setWorkflow] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const identity = scope ? githubWorkflowRunsIdentity(scope, workflow) : null
  const { owner, repo: repoName } = params
  const loadPage = useCallback(async (signal: AbortSignal, after?: string) => {
    const { github } = await loadRuns(
      { owner, repo: repoName, ...(workflow === null ? {} : { workflow }), ...(after ? { after } : {}) },
      signal,
    )
    if (!github) throw new Error('This repository no longer runs its checks on GitHub. Reload to see its runs.')
    return github
  }, [loadRuns, owner, repoName, workflow])
  const load = useCallback((signal: AbortSignal) => reloadGitHubWorkflowRunPages(
    (identity ? githubWorkflowRunsResource.peek(identity)?.pages : undefined) ?? 1,
    (after) => loadPage(signal, after),
  ), [identity, loadPage])
  const initialValue = useMemo(
    () => (workflow === null ? { list: initialRuns, pages: 1 } : null),
    [initialRuns, workflow],
  )
  const resource = useCachedResource({
    fallbackError: 'Runs could not refresh.',
    identity,
    initialValue,
    load,
    resource: githubWorkflowRunsResource,
  })
  const runs = resource.value?.list ?? initialValue?.list ?? null
  const rows = useMemo(() => runs?.workflow_runs.map(githubWorkflowRunRow) ?? [], [runs])
  const names = useCachedResource({
    fallbackError: 'Workflow names could not refresh.',
    identity: scope,
    initialValue: initialNames,
    load: useCallback((signal: AbortSignal) => loadNames({ owner, repo: repoName }, signal), [loadNames, owner, repoName]),
    resource: githubWorkflowNamesResource,
  })
  const workflows = githubWorkflowFilterOptions((names.value ?? initialNames).workflows, workflow)

  async function loadMore() {
    if (!identity) return
    setLoadingMore(true)
    try {
      await loadMoreGitHubWorkflowRuns(identity, (after, signal) => loadPage(signal, after))
    } finally {
      setLoadingMore(false)
    }
  }

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <div className="flex flex-wrap items-center gap-2">
            {workflows.length > 0 || workflow !== null ? (
              <select
                aria-label="Filter by workflow"
                className={SELECT_CLASS}
                onChange={(event) => setWorkflow(event.target.value === '' ? null : event.target.value)}
                value={workflow ?? ''}
              >
                <option value="">All workflows</option>
                {workflows.map((name) => <option key={name} value={name}>{name}</option>)}
              </select>
            ) : null}
            <Button asChild size="sm" variant="secondary">
              <a href={(runs ?? initialRuns).actions_url} rel="noopener noreferrer" target="_blank">
                <ExternalLink className="size-3.5" />
                <span>All runs on GitHub</span>
              </a>
            </Button>
          </div>
        )}
        summary="Workflow runs from GitHub Actions"
        title="Runs"
      />
      <div className="min-w-0 border-t border-border">
        <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
          {resource.error ? (
            <div className="pt-5">
              <PageErrorAlert title="Runs could not refresh">
                <div className="flex flex-wrap items-center gap-3">
                  <span>{resource.error}</span>
                  <Button onClick={resource.retry} size="sm" variant="secondary">
                    Retry now
                  </Button>
                </div>
              </PageErrorAlert>
            </div>
          ) : names.error ? (
            <div className="pt-5">
              <PageErrorAlert title="Workflow filter could not refresh">
                <div className="flex flex-wrap items-center gap-3">
                  <span>{names.error}</span>
                  <Button onClick={names.retry} size="sm" variant="secondary">
                    Retry now
                  </Button>
                </div>
              </PageErrorAlert>
            </div>
          ) : null}
          <div className="pt-7">
            {!runs ? (
              resource.error ? null : <GitHubWorkflowRunsSkeleton />
            ) : rows.length === 0 ? (
              workflow === null ? (
                <EmptyState
                  action={repo.access.actor !== 'Public' ? (
                    <Button asChild size="sm" variant="secondary">
                      <Link hash="ci" params={params} to="/$owner/$repo/settings">
                        <FlaskConical className="size-3.5" />
                        <span>Test connection</span>
                      </Link>
                    </Button>
                  ) : undefined}
                  description="Runs appear here once GitHub Actions starts a workflow for this repository."
                  icon={<TerminalSquare />}
                  title="No runs yet"
                />
              ) : (
                <EmptyState
                  description="GitHub has not reported a run of this workflow."
                  icon={<TerminalSquare />}
                  title={`No ${workflow} runs`}
                />
              )
            ) : (
              <>
                <ul className="divide-y divide-border">
                  {rows.map((row, index) => <GitHubWorkflowRunItem
                    key={row.key}
                    params={params}
                    row={row}
                    onOpen={() => {
                      if (scope && runs) seedGitHubWorkflowRunDetail(scope, runs.workflow_runs[index])
                    }}
                  />)}
                </ul>
                <div className="flex items-center justify-center gap-3 pt-5">
                  {runs.next_cursor ? (
                    <Button
                      aria-busy={loadingMore}
                      disabled={loadingMore}
                      onClick={() => void loadMore()}
                      variant="secondary"
                    >
                      {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
                      Load older runs
                    </Button>
                  ) : null}
                  <span className="text-xs text-muted-foreground">Showing {rows.length}</span>
                </div>
              </>
            )}
          </div>
        </main>
      </div>
    </WorkbenchPane>
  )
}

function GitHubWorkflowRunsSkeleton() {
  return (
    <ul aria-busy="true" aria-label="Loading runs" className="divide-y divide-border">
      {Array.from({ length: 4 }, (_, index) => (
        <li className={RUN_ROW_CLASS} key={index}>
          <TextSkeleton length="tiny" />
          <TextSkeleton className="flex-1" length="long" />
          <TextSkeleton length="short" />
        </li>
      ))}
    </ul>
  )
}

function GitHubWorkflowRunItem({ params, row, onOpen }: {
  params: RepoParams; row: GitHubWorkflowRunRow; onOpen: () => void
}) {
  return (
    <li className={cn(RUN_ROW_CLASS, row.state === 'running' && 'bg-info-soft/40')}>
      <RunStatusIcon state={row.state} />
      <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-baseline sm:gap-2">
        <Link
          className={cn('truncate text-sm font-medium', LINK_CLASS)}
          onClick={onOpen}
          params={{ ...params, runId: row.key }}
          to="/$owner/$repo/runs/$runId"
        >
          {row.name}
        </Link>
        <span className="truncate font-mono text-xs text-muted-foreground">
          #{row.commit}
          {row.branch ? (
            <>
              <span className="text-muted-foreground/70"> · </span>
              {row.requestId ? (
                <Link
                  className={LINK_CLASS}
                  params={{ ...params, requestId: row.requestId }}
                  title="Open the request"
                  to="/$owner/$repo/requests/$requestId"
                >
                  {row.branch}
                </Link>
              ) : row.branch}
            </>
          ) : null}
          <span className="text-muted-foreground/70"> · {row.event}</span>
        </span>
      </span>
      <span className="hidden shrink-0 text-xs text-muted-foreground sm:inline">{row.label}</span>
      <span className={`${RUN_ROW_TIMESTAMP_CLASS} text-right text-xs tabular-nums text-muted-foreground`}>
        <RelativeTimestamp value={row.at} />
      </span>
    </li>
  )
}
