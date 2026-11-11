import { useId, useMemo, useState } from 'react'
import type {
  StepLogs,
  StepSelection,
} from './repository-run-detail-controller'
import { RunEnvironmentPanel, RunEnvironmentToggle } from './run-attempt-environment'
import { RunJobHeader } from './run-job-header'
import { runLogLineId, type RunJobLink } from './run-job-ids'
import { lineByAnchor, logLinesText, readStepLogLines } from './run-log-lines'
import type { RunLogFocus } from './run-log-text'
import { startRunJobTiming } from './run-log-timing'
import { RunLogTools, useRunLogSearch } from './run-log-tools'
import { RunLogView } from './run-log-view'
import { RunStepRow } from './run-step-row'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunJobDetailResponse,
} from '@/api/types.generated'

const NO_LOGS: StepLogs['state']['logs'] = []

export function RunDetailSteps({
  attempt,
  jobDetail,
  link,
  onSelectAttempt,
  onSelectStep,
  runId,
  selection,
  stepLogs,
}: {
  attempt: RepositoryRunAttemptResponse | null
  jobDetail: RepositoryRunJobDetailResponse
  link: RunJobLink | null
  onSelectAttempt: (attemptId: string) => void
  onSelectStep: (attemptId: string, stepIndex: number) => void
  runId: string
  selection: StepSelection | null
  stepLogs: StepLogs
}) {
  const { job } = jobDetail
  const [environmentOpen, setEnvironmentOpen] = useState(false)
  const [wrap, setWrap] = useState(true)
  const environmentId = useId()
  const openStep = attempt && selection?.jobKey === job.key &&
    selection.attemptId === attempt.id
    ? selection.stepIndex
    : null
  const logs = openStep === null ? NO_LOGS : stepLogs.state.logs
  const text = useMemo(() => logs.map((log) => log.text).join(''), [logs])
  const lines = useMemo(() => readStepLogLines(logs), [logs])
  const search = useRunLogSearch(lines)
  const copyText = useMemo(() => logLinesText(lines), [lines])
  const [seenLink, setSeenLink] = useState(link)
  const [marked, setMarked] = useState(() => linkedLine(link, job.key))
  const [linkFocused, setLinkFocused] = useState(true)
  if (link !== seenLink) {
    setSeenLink(link)
    setMarked(linkedLine(link, job.key))
    setLinkFocused(true)
  }
  const shownLine = (target: LinkedLine | null) =>
    target && attempt && target.attempt === attempt.id && target.step === openStep
      ? lineByAnchor(lines, target.anchor)?.number ?? null
      : null
  const markedLine = shownLine(marked)
  const linkFocusLine = linkFocused ? shownLine(linkedLine(link, job.key)) : null
  const linkFocus = useMemo<RunLogFocus | null>(
    () => (linkFocusLine === null ? null : { line: linkFocusLine }),
    [linkFocusLine],
  )

  return (
    <>
      <RunJobHeader
        attempt={attempt}
        environmentControl={attempt ? (
          <RunEnvironmentToggle
            caches={attempt.caches}
            expanded={environmentOpen}
            onToggle={() => setEnvironmentOpen((open) => !open)}
            panelId={environmentId}
          />
        ) : null}
        jobDetail={jobDetail}
        onSelectAttempt={onSelectAttempt}
        tools={openStep === null ? null : (
          <RunLogTools
            copyText={copyText}
            onToggleWrap={() => setWrap((value) => !value)}
            search={search}
            wrap={wrap}
          />
        )}
      />
      {attempt && environmentOpen ? (
        <RunEnvironmentPanel
          cacheSetup={attempt.cache_setup}
          caches={attempt.caches}
          id={environmentId}
          pinnedContainerImage={job.pinned_container_image}
        />
      ) : null}
      <div className="lg:min-h-0 lg:flex-1 lg:overflow-y-auto">
        {attempt ? (
          attempt.steps.length === 0 ? (
            <p className="px-4 py-5 text-sm text-muted-foreground">
              Steps are created when a runner claims this run.
            </p>
          ) : (
            <ol aria-label="Steps">
              {attempt.steps.map((step) => (
                <RunStepRow
                  completedAt={step.completed_at_unix}
                  exitCode={step.exit_code}
                  key={step.index}
                  name={step.name}
                  onToggle={() => {
                    if (openStep !== step.index) startRunJobTiming(runId)
                    onSelectStep(attempt.id, step.index)
                  }}
                  open={openStep === step.index}
                  panelId={`run-step-${attempt.id}-${step.index}`}
                  startedAt={step.started_at_unix}
                  state={step.state}
                >
                  <RunLogView
                    focus={search.focus ?? linkFocus}
                    lineId={(line) => runLogLineId(job.key, attempt.id, step.index, line.anchor)}
                    lines={lines}
                    logs={stepLogs}
                    markedLine={markedLine}
                    onFollowTail={() => {
                      search.release()
                      setLinkFocused(false)
                    }}
                    onLinkLine={(line) => {
                      window.history.replaceState(window.history.state, '', `#${runLogLineId(job.key, attempt.id, step.index, line.anchor)}`)
                      setMarked({ attempt: attempt.id, step: step.index, anchor: line.anchor })
                    }}
                    runId={runId}
                    search={search}
                    step={step}
                    text={text}
                    wrap={wrap}
                  />
                </RunStepRow>
              ))}
            </ol>
          )
        ) : (
          <p className="px-4 py-5 text-sm text-muted-foreground">
            {job.state === 'blocked'
              ? 'Waiting for required jobs to finish.'
              : job.state === 'queued'
                ? 'Waiting for cloud capacity.'
                : 'No attempts were created for this job.'}
          </p>
        )}
      </div>
    </>
  )
}

type LinkedLine = { attempt: string; step: number; anchor: string }

function linkedLine(link: RunJobLink | null, jobKey: string): LinkedLine | null {
  return link && link.key === jobKey && link.attempt !== null && link.step !== null && link.line !== null
    ? { attempt: link.attempt, step: link.step, anchor: link.line }
    : null
}
