import type { RunActionInput } from '@/api/types'
import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunResponse,
} from '@/api/types.generated'
import { WorkbenchPane } from '@/components/page-header'
import { SeparatedMetaItem } from '@/components/separated-meta'
import { RelativeTimestamp } from '@/components/timestamp'
import { buttonVariants } from '@/components/ui/button-variants'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { githubRunResult } from './github-run-status'
import { GitHubWorkflowJobPane } from './github-workflow-job-pane'
import { githubJobKey, githubRunCanChange } from './github-workflow-run-detail-model'
import {
  githubWorkflowRunDetailIdentity,
  githubWorkflowRunDetailResource,
  recordingFetch,
  useGitHubWorkflowRunRecheck,
} from './github-workflow-run-detail-resource'
import { RunDuration } from './run-duration'
import { startRunJobTiming, useRunLogAbsence, useRunLogTimingPage } from './run-log-timing'
import { useRunJobHash } from './run-job-hash'
import { runJobLinkForHash, runJobPanelId, type RunJobLink } from './run-job-ids'
import { RunJobTree } from './run-job-tree'
import { RUN_BODY_CLASS } from './run-layout'
import { runDurationLead } from './run-status'
import { RunDetailPageError } from './repository-run-detail-page'
import { RunDetailPagePending } from './run-detail-pending'
import { RunHeaderStatus, RunPageHeader } from './run-detail-header'

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
  useRunLogTimingPage(params.run_id)
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const identity = scope ? githubWorkflowRunDetailIdentity(scope, params.run_id) : null
  const pageDetail = initialDetail && initialScope === scope ? initialDetail : null
  const load = useMemo(() => recordingFetch(loadDetail), [loadDetail])
  const resource = useCachedResource({
    fallbackError: 'The run could not refresh.',
    identity,
    initialValue: pageDetail,
    load,
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
  const [link, setLink] = useState<RunJobLink | null>(null)
  useRunJobHash((hash) => {
    const linked = runJobLinkForHash(detail.jobs.map(githubJobKey), hash)
    if (linked === null) return null
    setChosenKey(linked.key)
    setLink(linked)
    return linked.key
  })
  const selectedJob = detail.jobs.find((job) => githubJobKey(job) === chosenKey)
    ?? detail.jobs.find((job) => job.id === detail.opening_job_id)
    ?? null
  useRunLogAbsence(params.run_id, selectedJob === null && !detail.jobs_not_read_yet)
  const summaries = useMemo(() => detail.jobs.map((job) => ({
    key: githubJobKey(job),
    name: job.name,
    state: githubRunResult(job.status, job.conclusion).state,
    startedAt: job.started_at_unix,
    completedAt: job.completed_at_unix,
  })), [detail.jobs])

  function pickJob(key: string) {
    const job = detail.jobs.find((listed) => githubJobKey(listed) === key)
    if (!job) return
    if (job !== selectedJob) startRunJobTiming(params.run_id)
    setChosenKey(key)
    window.history.replaceState(window.history.state, '', `#${runJobPanelId(key)}`)
  }

  return (
    <WorkbenchPane className="flex flex-col lg:h-[calc(100dvh-var(--app-topbar))]">
      <GitHubWorkflowRunHeader
        error={error}
        jobStates={summaries.map((job) => job.state)}
        onRetry={onRetry}
        params={params}
        run={detail.run}
      />
      {detail.jobs.length === 0 ? (
        <p aria-busy={detail.jobs_not_read_yet} className="border-t border-border px-4 py-6 text-sm text-muted-foreground">
          {detail.jobs_not_read_yet ? 'Loading jobs…' : 'Jobs appear here once the run starts.'}
        </p>
      ) : (
        <div className={RUN_BODY_CLASS}>
          <RunJobTree jobs={summaries} onSelect={pickJob} selectedKey={selectedJob ? githubJobKey(selectedJob) : null} />
          {selectedJob ? (
            <div
              className="flex min-w-0 flex-col lg:min-h-0"
              id={runJobPanelId(githubJobKey(selectedJob))}
            >
              <GitHubWorkflowJobPane
                includedLog={selectedJob.id === detail.opening_job_id ? detail.opening_job_log : null}
                job={selectedJob}
                key={githubJobKey(selectedJob)}
                link={link?.key === githubJobKey(selectedJob) ? link : null}
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
  jobStates,
  onRetry,
  params,
  run,
}: {
  error: string | null
  jobStates: readonly string[]
  onRetry: () => void
  params: RunActionInput
  run: GitHubWorkflowRunResponse
}) {
  const result = githubRunResult(run.status, run.conclusion)
  const started = run.run_started_at_unix

  return (
    <RunPageHeader
      action={(
        <a
          className={buttonVariants({ size: 'sm', variant: 'secondary' })}
          href={run.html_url}
          rel="noopener noreferrer"
          target="_blank"
        >
          Open on GitHub
        </a>
      )}
      error={error}
      jobStates={jobStates}
      name={run.workflow_name}
      onRetry={onRetry}
      params={params}
      state={result.state}
    >
      <SeparatedMetaItem>
        <RunHeaderStatus state={result.state}>
          {started === null ? (
            <span className="first-letter:uppercase">{result.label}</span>
          ) : (
            <>
              {runDurationLead(result.state, result.label)}{' '}
              <RunDuration end={run.status === 'completed' ? run.updated_at_unix : null} start={started} />
            </>
          )}
        </RunHeaderStatus>
      </SeparatedMetaItem>
      {run.branch ? (
        <SeparatedMetaItem>
          {run.request_id ? (
            <Link
              className="underline underline-offset-2 hover:text-foreground"
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
      <SeparatedMetaItem>{run.event.replaceAll('_', ' ')}</SeparatedMetaItem>
      {started === null ? null : (
        <SeparatedMetaItem>
          started <RelativeTimestamp value={started} />
        </SeparatedMetaItem>
      )}
    </RunPageHeader>
  )
}
