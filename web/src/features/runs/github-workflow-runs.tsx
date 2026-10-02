import type { RepoParams } from '@/api/types'
import type {
  GitHubWorkflowRunListResponse,
  GitHubWorkflowRunsResponse,
} from '@/api/types.generated'
import { EmptyState } from '@/components/empty-state'
import { PageErrorAlert } from '@/components/page-error-alert'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { useCachedResource } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { ExternalLink, TerminalSquare } from 'lucide-react'
import { useCallback } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { type GitHubWorkflowRunRow, githubWorkflowRunRow } from './github-workflow-run-model'
import { githubWorkflowRunsResource } from './github-workflow-runs-resource'
import { RunStatusIcon } from './run-status-icon'
import { RUN_ROW_CLASS, RUN_ROW_TIMESTAMP_CLASS } from './run-row-layout'

const LINK_CLASS = 'underline-offset-2 hover:text-foreground hover:underline'

/**
 * The Runs page of a repository whose checks run on GitHub. It lists the
 * workflow runs GitHub reported and links each to GitHub, which keeps the
 * logs. Repository events refresh the list in place.
 */
export function GitHubWorkflowRunsPage({
  initialRuns,
  loadRuns,
  params,
}: {
  initialRuns: GitHubWorkflowRunListResponse
  loadRuns: (params: RepoParams, signal: AbortSignal) => Promise<GitHubWorkflowRunsResponse>
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const load = useCallback(async (signal: AbortSignal) => {
    const { github } = await loadRuns({ owner, repo: repoName }, signal)
    if (!github) throw new Error('This repository no longer runs its checks on GitHub. Reload to see its runs.')
    return github
  }, [loadRuns, owner, repoName])
  const resource = useCachedResource({
    fallbackError: 'Runs could not refresh.',
    identity: scope,
    initialValue: initialRuns,
    load,
    resource: githubWorkflowRunsResource,
  })
  const runs = resource.value ?? initialRuns
  const rows = runs.workflow_runs.map(githubWorkflowRunRow)

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <Button asChild size="sm" variant="secondary">
            <a href={runs.actions_url} rel="noopener noreferrer" target="_blank">
              <ExternalLink className="size-3.5" />
              <span>All runs on GitHub</span>
            </a>
          </Button>
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
          ) : null}
          <div className="pt-7">
            {rows.length === 0 ? (
              <EmptyState
                description="Runs appear here once GitHub Actions starts a workflow for this repository."
                icon={<TerminalSquare />}
                title="No runs yet"
              />
            ) : (
              <ul className="divide-y divide-border">
                {rows.map((row) => <GitHubWorkflowRunItem key={row.key} params={params} row={row} />)}
              </ul>
            )}
          </div>
        </main>
      </div>
    </WorkbenchPane>
  )
}

function GitHubWorkflowRunItem({ params, row }: { params: RepoParams; row: GitHubWorkflowRunRow }) {
  return (
    <li className={cn(RUN_ROW_CLASS, row.state === 'running' && 'bg-info-soft/40')}>
      <RunStatusIcon state={row.state} />
      <span className="flex min-w-0 flex-1 flex-col sm:flex-row sm:items-baseline sm:gap-2">
        <a
          className={cn('truncate text-sm font-medium', LINK_CLASS)}
          href={row.href}
          rel="noopener noreferrer"
          target="_blank"
        >
          {row.name}
        </a>
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
