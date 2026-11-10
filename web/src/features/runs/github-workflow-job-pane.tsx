import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowJobResponse,
} from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { useCallback, useMemo, useState, type ReactNode } from 'react'
import { githubRunResult } from './github-run-status'
import {
  githubJobFailedStep,
  githubJobKey,
  githubJobLog,
  githubJobLogAbsence,
  githubOpeningStep,
} from './github-workflow-run-detail-model'
import {
  githubWorkflowJobLogIdentity,
  githubWorkflowJobLogResource,
} from './github-workflow-run-detail-resource'
import { RunDuration } from './run-duration'
import { runJobPanelId, runLogLineId, type RunJobLink } from './run-job-ids'
import { lineByNumber, logLinesText, type LogLine } from './run-log-lines'
import { RunLogPlaceholder, RunLogText, type RunLogFocus } from './run-log-text'
import { observeRunLogFetching, useRunLogPaintTiming } from './run-log-timing'
import { RunLogTools, useRunLogSearch } from './run-log-tools'
import { RunJobHeading } from './run-job-header'
import { RunStepRow } from './run-step-row'

const LINK_CLASS = 'underline underline-offset-2 hover:text-foreground'
const NO_LINES: readonly LogLine[] = []

type FocusRequest = number | 'first-error' | null

export function GitHubWorkflowJobPane({
  includedLog,
  job,
  link,
  loadLog,
  runId,
  scope,
}: {
  includedLog: GitHubWorkflowJobLogResponse | null
  job: GitHubWorkflowJobResponse
  link: RunJobLink | null
  loadLog: (jobId: string, signal: AbortSignal) => Promise<GitHubWorkflowJobLogResponse>
  runId: string
  scope: string | null
}) {
  const key = githubJobKey(job)
  const finished = job.status === 'completed'
  const load = useCallback(async (signal: AbortSignal) => {
    if (includedLog && includedLog.state !== 'pending') return includedLog
    const answer = await loadLog(key, signal)
    if (answer.state === 'pending' || (answer.state === 'kept' && answer.fetched)) observeRunLogFetching(runId)
    return answer
  }, [includedLog, key, loadLog, runId])
  const log = useCachedResource({
    fallbackError: 'The log could not load.',
    identity: scope && finished ? githubWorkflowJobLogIdentity(scope, runId, key) : null,
    initialValue: includedLog,
    load,
    resource: githubWorkflowJobLogResource,
  })
  useRetryOnReconnect(log)
  const [wrap, setWrap] = useState(true)
  const [times, setTimes] = useState(false)
  const result = githubRunResult(job.status, job.conclusion)
  const failedStep = githubJobFailedStep(job)
  const logText = log.value?.state === 'kept' ? log.value.text : null
  const parsed = useMemo(() => logText === null ? null : githubJobLog(logText, job.steps), [job.steps, logText])
  const search = useRunLogSearch(parsed?.lines ?? NO_LINES)

  const [seenLink, setSeenLink] = useState(link)
  const [openSteps, setOpenSteps] = useState<ReadonlySet<number>>(() => openingSteps(job, link))
  const [markedLine, setMarkedLine] = useState(link?.line ?? null)
  const [focusRequest, setFocusRequest] = useState<FocusRequest>(link?.line ?? (failedStep ? 'first-error' : null))
  if (link !== seenLink) {
    setSeenLink(link)
    const linkedStep = link?.step ?? null
    if (linkedStep !== null) setOpenSteps((current) => new Set([...current, linkedStep]))
    setMarkedLine(link?.line ?? null)
    setFocusRequest(link?.line ?? null)
  }
  if (search.query && focusRequest !== null) setFocusRequest(null)

  const focus = useMemo<RunLogFocus | null>(() => {
    if (!parsed || focusRequest === null) return null
    if (focusRequest !== 'first-error') return { line: focusRequest }
    const failedLines = failedStep ? parsed.stepLines.get(failedStep.number) ?? [] : []
    const firstError = failedLines.find((line) => line.kind === 'error')
    return firstError ? { line: firstError.number } : null
  }, [failedStep, focusRequest, parsed])
  const activeFocus = search.focus ?? focus
  const matchStep = search.current && parsed ? lineByNumber(parsed.lines, search.current.line)?.step : null
  const shownSteps = matchStep == null || openSteps.has(matchStep) ? openSteps : new Set([...openSteps, matchStep])
  const shownKey = [...shownSteps].sort().join(',')
  const visibleText = useMemo(() => {
    if (!parsed) return ''
    if (job.steps.length === 0) return logLinesText(parsed.lines)
    const shown = new Set(shownKey.split(',').map(Number))
    return job.steps.filter((step) => shown.has(step.number)).map((step) => logLinesText(parsed.stepLines.get(step.number) ?? NO_LINES)).join('\n')
  }, [job.steps, parsed, shownKey])
  const copyText = useMemo(() => (parsed ? logLinesText(parsed.lines) : ''), [parsed])
  useRunLogPaintTiming({
    provider: 'github',
    runId,
    settled: !finished || (log.value ? log.value.state !== 'pending' : log.error !== null),
    text: visibleText,
  })

  function linkLine(step: number, line: number) {
    window.history.replaceState(window.history.state, '', `#${runLogLineId(key, step, line)}`)
    setMarkedLine(line)
  }

  function toggleStep(step: number) {
    setOpenSteps((current) => {
      const next = new Set(current)
      if (!next.delete(step)) next.add(step)
      return next
    })
  }

  function stepBody(step: number, lines: readonly LogLine[]): ReactNode {
    if (!finished) return <LogNote>The log appears when this job finishes.</LogNote>
    if (log.error && !log.value) {
      return (
        <p className="flex flex-wrap items-center gap-3 px-4 py-2 text-sm text-danger-strong" role="alert">
          {log.error}
          <Button onClick={log.retry} size="sm" variant="secondary">Retry</Button>
        </p>
      )
    }
    if (!log.value || log.value.state === 'pending') return <RunLogPlaceholder />
    if (log.value.state !== 'kept') return <LogNote>{githubJobLogAbsence(log.value, job)}</LogNote>
    return (
      <RunLogText
        currentMatch={search.current}
        empty="No output."
        focus={activeFocus}
        followTail={false}
        lineId={(line) => runLogLineId(key, step, line)}
        lines={lines}
        markedLine={markedLine}
        matches={search.byLine}
        onLinkLine={(line) => linkLine(step, line)}
        times={times}
        wrap={wrap}
      />
    )
  }

  return (
    <>
      <RunJobHeading
        detail={result.state === 'failed' && (failedStep || result.label !== 'failed') ? (
          <>
            <span className="font-medium text-danger-strong">{result.label}</span>
            {failedStep ? ` at ${failedStep.name}` : null}
            {job.started_at_unix === null ? null : <> · <RunDuration end={job.completed_at_unix} start={job.started_at_unix} /></>}
          </>
        ) : job.started_at_unix === null ? null : <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />}
        name={job.name}
        state={result.state}
        tools={parsed ? (
          <RunLogTools
            copyText={copyText}
            onToggleTimes={() => setTimes((value) => !value)}
            onToggleWrap={() => setWrap((value) => !value)}
            search={search}
            times={times}
            wrap={wrap}
          />
        ) : null}
      />
      <div className="lg:min-h-0 lg:flex-1 lg:overflow-y-auto">
        {log.value?.state === 'kept' && log.value.truncated ? (
          <p className="border-b border-border px-4 py-2 text-xs text-muted-foreground">
            Showing the end of a long log.{' '}
            <a className={LINK_CLASS} href={job.html_url} rel="noopener noreferrer" target="_blank">Full log</a>
          </p>
        ) : null}
        {job.steps.length > 0 ? (
          <ol aria-label="Steps">
            {job.steps.map((step) => {
              const state = githubRunResult(step.status, step.conclusion).state
              return (
                <RunStepRow
                  completedAt={step.completed_at_unix}
                  exitCode={null}
                  key={step.number}
                  name={step.name}
                  onToggle={() => toggleStep(step.number)}
                  open={shownSteps.has(step.number)}
                  panelId={`${runJobPanelId(key)}-step-${step.number}`}
                  startedAt={step.started_at_unix}
                  state={state}
                >
                  {stepBody(step.number, parsed?.stepLines.get(step.number) ?? NO_LINES)}
                </RunStepRow>
              )
            })}
          </ol>
        ) : (
          <>
            <LogNote>{finished ? 'No steps ran.' : 'Steps appear once a runner picks up this job.'}</LogNote>
            {finished ? <section aria-label="Log">{stepBody(0, parsed?.lines ?? NO_LINES)}</section> : null}
          </>
        )}
      </div>
    </>
  )
}

function openingSteps(job: GitHubWorkflowJobResponse, link: RunJobLink | null): ReadonlySet<number> {
  const step = link?.step ?? githubOpeningStep(job)
  return new Set(step === null ? [] : [step])
}

function LogNote({ children }: { children: ReactNode }) {
  return <p className="px-4 py-2.5 text-sm text-muted-foreground">{children}</p>
}
