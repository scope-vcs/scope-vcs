import type { RunActionInput } from '@/api/types'
import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowJobResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunResponse,
} from '@/api/types.generated'
import { WorkbenchPane } from '@/components/page-header'
import { SeparatedMeta, SeparatedMetaItem } from '@/components/separated-meta'
import { formatUnixDate, formatUnixDateUtc } from '@/lib/date-format'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useHydrated } from '@/lib/use-hydrated'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { githubRunResult } from './github-run-status'
import { GitHubWorkflowJobPane } from './github-workflow-job-pane'
import {
  githubJobKey,
  githubRunCanChange,
  selectGitHubJob,
} from './github-workflow-run-detail-model'
import {
  githubWorkflowRunDetailIdentity,
  githubWorkflowRunDetailResource,
  useGitHubWorkflowRunRecheck,
} from './github-workflow-run-detail-resource'
import { RunDuration } from './run-duration'
import { useRunJobHash } from './run-job-hash'
import { jobKeyForHash, runJobPanelId } from './run-job-ids'
import { RUN_JOB_ITEM_CLASS, RUN_JOB_LIST_CLASS, RUN_LINK_CLASS, runJobButtonClass } from './run-layout'
import { RunStatusIcon } from './run-status-icon'
import { runDurationLead } from './run-status'
import { RunDetailPageError } from './repository-run-detail-page'
import { RunDetailPagePending } from './run-detail-pending'
import {
  LiveUpdatesPaused,
  RunHeaderStatus,
  RunHeaderTitle,
} from './run-detail-header'

type GitHubWorkflowRunDetailPageProps = {
  initialDetail: GitHubWorkflowRunDetailResponse | null
  initialScope: string | null
  loadDetail: (signal: AbortSignal) => Promise<GitHubWorkflowRunDetailResponse>
  loadLog: (jobId: string, signal: AbortSignal) => Promise<GitHubWorkflowJobLogResponse>
  params: RunActionInput
}

export function GitHubWorkflowRunDetailPage({
  initialDetail,
  initialScope,
  loadDetail,
  loadLog,
  params,
}: GitHubWorkflowRunDetailPageProps) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const identity = scope ? githubWorkflowRunDetailIdentity(scope, params.run_id) : null
  const pageDetail = initialDetail && initialScope === scope ? initialDetail : null
  const resource = useCachedResource({
    fallbackError: 'The run could not refresh.',
    identity,
    initialValue: pageDetail,
    load: loadDetail,
    resource: githubWorkflowRunDetailResource,
  })
  if (!resource.value) {
    return resource.error ? <RunDetailPageError error={resource.error} /> : <RunDetailPagePending />
  }
  return (
    <GitHubWorkflowRunView
      detail={resource.value}
      error={resource.error}
      identity={identity}
      loadLog={loadLog}
      onRetry={resource.retry}
      pageDetail={pageDetail}
      params={params}
      scope={scope}
    />
  )
}

function GitHubWorkflowRunView({
  detail,
  error,
  identity,
  loadLog,
  onRetry,
  pageDetail,
  params,
  scope,
}: {
  detail: GitHubWorkflowRunDetailResponse
  error: string | null
  identity: string | null
  loadLog: GitHubWorkflowRunDetailPageProps['loadLog']
  onRetry: () => void
  pageDetail: GitHubWorkflowRunDetailResponse | null
  params: RunActionInput
  scope: string | null
}) {
  useGitHubWorkflowRunRecheck(identity, githubRunCanChange(detail), pageDetail)

  const [chosenKey, setChosenKey] = useState<string | null>(null)
  useRunJobHash((hash) => {
    const key = jobKeyForHash(detail.jobs.map(githubJobKey), hash)
    if (key !== null) setChosenKey(key)
    return key !== null
  })
  const selectedJob = selectGitHubJob(detail.jobs, chosenKey)

  function pickJob(job: GitHubWorkflowJobResponse) {
    const key = githubJobKey(job)
    setChosenKey(key)
    window.history.replaceState(window.history.state, '', `#${runJobPanelId(key)}`)
  }

  return (
    <WorkbenchPane className="flex flex-col lg:h-[calc(100dvh-var(--app-topbar))]">
      <GitHubWorkflowRunHeader
        error={error}
        onRetry={onRetry}
        params={params}
        run={detail.run}
      />
      {detail.jobs.length === 0 ? (
        <p aria-busy={detail.jobs_not_read_yet} className="border-t border-border px-4 py-6 text-sm text-muted-foreground">
          {detail.jobs_not_read_yet ? 'Loading jobs…' : 'Jobs appear here once the run starts.'}
        </p>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col border-t border-border lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)]">
          <nav
            aria-label="Jobs"
            className="flex min-w-0 items-center border-b border-border lg:block lg:overflow-y-auto lg:border-b-0 lg:border-r lg:py-2"
          >
            <ul className={RUN_JOB_LIST_CLASS}>
              {detail.jobs.map((job) => {
                const key = githubJobKey(job)
                const selected = job === selectedJob
                return (
                  <li className={RUN_JOB_ITEM_CLASS} key={key}>
                    <button
                      aria-controls={runJobPanelId(key)}
                      aria-pressed={selected}
                      className={runJobButtonClass(selected)}
                      onClick={() => pickJob(job)}
                      type="button"
                    >
                      <RunStatusIcon state={githubRunResult(job.status, job.conclusion).state} />
                      <span className="min-w-0 flex-1 truncate">{job.name}</span>
                      {job.started_at_unix === null ? null : (
                        <span className="text-xs font-normal text-muted-foreground">
                          <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />
                        </span>
                      )}
                    </button>
                  </li>
                )
              })}
            </ul>
          </nav>
          {selectedJob ? (
            <div
              className="flex min-w-0 flex-col lg:min-h-0"
              id={runJobPanelId(githubJobKey(selectedJob))}
            >
              <GitHubWorkflowJobPane
                job={selectedJob}
                key={githubJobKey(selectedJob)}
                loadLog={loadLog}
                runId={params.run_id}
                scope={scope}
              />
            </div>
          ) : null}
        </div>
      )}
    </WorkbenchPane>
  )
}

function GitHubWorkflowRunHeader({
  error,
  onRetry,
  params,
  run,
}: {
  error: string | null
  onRetry: () => void
  params: RunActionInput
  run: GitHubWorkflowRunResponse
}) {
  const result = githubRunResult(run.status, run.conclusion)
  const hydrated = useHydrated()
  const formatDate = hydrated ? formatUnixDate : formatUnixDateUtc
  const started = run.run_started_at_unix

  return (
    <header className="px-4 py-3">
      <RunHeaderTitle name={run.workflow_name} params={params} />
      <SeparatedMeta className="mt-0.5 text-xs text-muted-foreground">
        <SeparatedMetaItem>
          <RunHeaderStatus
            state={result.state}
            title={started === null ? undefined : `Started ${formatDate(started)}`}
          >
            {started === null ? (
              <span className="first-letter:uppercase">{result.label}</span>
            ) : (
              <span>
                {runDurationLead(result.state, result.label)}{' '}
                <RunDuration end={run.status === 'completed' ? run.updated_at_unix : null} start={started} />
              </span>
            )}
          </RunHeaderStatus>
        </SeparatedMetaItem>
        {run.branch ? (
          <SeparatedMetaItem>
            {run.request_id ? (
              <Link
                className={RUN_LINK_CLASS}
                params={{ owner: params.owner, repo: params.repo, requestId: run.request_id }}
                title="Open the request"
                to="/$owner/$repo/requests/$requestId"
              >
                {run.branch}
              </Link>
            ) : run.branch}
          </SeparatedMetaItem>
        ) : null}
        <SeparatedMetaItem>
          <code>{run.head_oid.slice(0, 7)}</code>
        </SeparatedMetaItem>
        <SeparatedMetaItem>{run.event}</SeparatedMetaItem>
      </SeparatedMeta>
      <LiveUpdatesPaused error={error} onRetry={onRetry} />
    </header>
  )
}
