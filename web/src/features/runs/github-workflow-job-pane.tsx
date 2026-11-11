import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowJobResponse,
  GitHubWorkflowStepResponse,
} from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { useCallback, useMemo, useState } from 'react'
import { githubRunResult } from './github-run-status'
import {
  githubJobFailedStep,
  githubJobKey,
  githubJobLogAbsence,
  stripLogTimestamps,
} from './github-workflow-run-detail-model'
import {
  githubWorkflowJobLogIdentity,
  githubWorkflowJobLogResource,
} from './github-workflow-run-detail-resource'
import { RunDuration } from './run-duration'
import { RunLogControls } from './run-job-header'
import { RUN_LOG_TEXT_CLASS, RUN_STEP_ROW_CLASS } from './run-layout'
import { RunLogText } from './run-log-text'
import { observeRunLogFetching, useRunLogPaintTiming } from './run-log-timing'
import { RunStatusIcon } from './run-status-icon'

const LINK_CLASS = 'underline underline-offset-2 hover:text-foreground'

export function GitHubWorkflowJobPane({
  includedLog,
  job,
  loadLog,
  runId,
  scope,
}: {
  includedLog: GitHubWorkflowJobLogResponse | null
  job: GitHubWorkflowJobResponse
  loadLog: (jobId: string, signal: AbortSignal) => Promise<GitHubWorkflowJobLogResponse>
  runId: string
  scope: string | null
}) {
  const key = githubJobKey(job)
  const finished = job.status === 'completed'
  const load = useCallback(async (signal: AbortSignal) => {
    if (includedLog && includedLog.state !== 'pending') return readableJobLog(includedLog)
    const answer = await loadLog(key, signal)
    if (answer.state === 'pending' || (answer.state === 'kept' && answer.fetched)) observeRunLogFetching(runId)
    return readableJobLog(answer)
  }, [includedLog, key, loadLog, runId])
  const initialValue = useMemo(() => includedLog && readableJobLog(includedLog), [includedLog])
  const log = useCachedResource({
    fallbackError: 'The log could not load.',
    identity: scope && finished ? githubWorkflowJobLogIdentity(scope, runId, key) : null,
    initialValue,
    load,
    resource: githubWorkflowJobLogResource,
  })
  useRetryOnReconnect(log)
  const [wrap, setWrap] = useState(true)
  const result = githubRunResult(job.status, job.conclusion)
  const failedStep = githubJobFailedStep(job)
  const logText = log.value?.state === 'kept' ? log.value.text : null
  useRunLogPaintTiming({
    provider: 'github',
    runId,
    settled: !finished || (log.value ? log.value.state !== 'pending' : log.error !== null),
    text: logText ?? '',
  })

  return (
    <>
      <div className="flex min-h-12 flex-none flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-border py-2 pl-4 pr-3">
        <span className="flex min-w-0 max-w-full items-center gap-2.5">
          <RunStatusIcon state={result.state} />
          <h2 className="min-w-0 truncate text-[15px] font-semibold">{job.name}</h2>
        </span>
        {result.state === 'failed' && (failedStep || result.label !== 'failed') ? (
          <span className="min-w-0 truncate text-xs text-muted-foreground">
            <span className="text-danger-strong">{result.label}</span>
            {failedStep ? ` at ${failedStep.name}` : null}
          </span>
        ) : null}
        <span className="ml-auto flex items-center gap-1">
          {job.started_at_unix === null ? null : (
            <span className="mr-1 text-xs text-muted-foreground">
              <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />
            </span>
          )}
          {logText ? (
            <RunLogControls onToggleWrap={() => setWrap((value) => !value)} text={logText} wrap={wrap} />
          ) : null}
        </span>
      </div>
      <div className="lg:min-h-0 lg:flex-1 lg:overflow-y-auto">
        {job.steps.length > 0 ? (
          <ol aria-label="Steps">
            {job.steps.map((step) => <GitHubStepRow key={step.number} step={step} />)}
          </ol>
        ) : (
          <p className="border-b border-border px-4 py-5 text-sm text-muted-foreground">
            {finished ? 'No steps ran.' : 'Steps appear once a runner picks up this job.'}
          </p>
        )}
        <section aria-label="Log" className={cn('px-4 pt-3', RUN_LOG_TEXT_CLASS)}>
          {!finished ? (
            <p className="pb-4 font-sans text-muted-foreground">The log appears when this job finishes.</p>
          ) : log.error && !log.value ? (
            <p className="flex flex-wrap items-center gap-3 pb-4 font-sans text-danger-strong" role="alert">
              {log.error}
              <Button onClick={log.retry} size="sm" variant="secondary">
                Retry
              </Button>
            </p>
          ) : !log.value || log.value.state === 'pending' ? (
            <p className="pb-4 font-sans text-muted-foreground">Loading log…</p>
          ) : log.value.state !== 'kept' ? (
            <p className="pb-4 font-sans text-muted-foreground">{githubJobLogAbsence(log.value, job)}</p>
          ) : (
            <>
              {log.value.truncated ? (
                <p className="font-sans text-muted-foreground">
                  Showing the end of a long log.{' '}
                  <a className={LINK_CLASS} href={job.html_url} rel="noopener noreferrer" target="_blank">
                    Full log
                  </a>
                </p>
              ) : null}
              <RunLogText empty="No output." text={log.value.text} wrap={wrap} />
            </>
          )}
        </section>
      </div>
    </>
  )
}

function readableJobLog(log: GitHubWorkflowJobLogResponse): GitHubWorkflowJobLogResponse {
  return log.state === 'kept' ? { ...log, text: stripLogTimestamps(log.text) } : log
}

function GitHubStepRow({ step }: { step: GitHubWorkflowStepResponse }) {
  const result = githubRunResult(step.status, step.conclusion)
  return (
    <li className={`${RUN_STEP_ROW_CLASS} border-b border-border`}>
      <span aria-hidden="true" className="size-3.5" />
      <RunStatusIcon state={result.state} />
      <span className="truncate text-sm font-medium">{step.name}</span>
      <span className="text-xs text-muted-foreground">
        {result.state === 'skipped'
          ? 'Skipped'
          : <RunDuration end={step.completed_at_unix} start={step.started_at_unix} />}
      </span>
    </li>
  )
}
