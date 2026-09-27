import { cn } from '@/lib/utils'
import { ChevronRight } from 'lucide-react'
import { useId, useState } from 'react'
import type {
  StepLogs,
  StepSelection,
} from './repository-run-detail-controller'
import { RunEnvironmentPanel, RunEnvironmentToggle } from './run-attempt-environment'
import { RunDuration } from './run-duration'
import { RunJobHeader } from './run-job-header'
import { RunLogView } from './run-log-view'
import { RunStatusIcon } from './run-status-icon'
import { RUN_STEP_ROW_CLASS } from './run-step-layout'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunJobDetailResponse,
  RepositoryRunStepResponse,
} from '@/api/types.generated'

/** The selected job: its one-line header, the environment panel when asked
 * for, then its steps in the pane that scrolls. */
export function RunDetailSteps({
  attempt,
  jobDetail,
  onSelectAttempt,
  onSelectStep,
  selection,
  stepLogs,
}: {
  attempt: RepositoryRunAttemptResponse | null
  jobDetail: RepositoryRunJobDetailResponse
  onSelectAttempt: (attemptId: string) => void
  onSelectStep: (attemptId: string, stepIndex: number) => void
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
        logText={openStep === null ? null : stepLogs.state.logs.map((log) => log.text).join('')}
        onSelectAttempt={onSelectAttempt}
        onToggleWrap={() => setWrap((value) => !value)}
        wrap={wrap}
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
          <>
            {attempt.steps.map((step) => (
              <StepRow
                attemptId={attempt.id}
                key={step.index}
                onSelect={() => onSelectStep(attempt.id, step.index)}
                selected={openStep === step.index}
                step={step}
                stepLogs={stepLogs}
                wrap={wrap}
              />
            ))}
            {attempt.steps.length === 0 ? (
              <p className="px-4 py-5 text-sm text-muted-foreground">
                Steps are created when a runner claims this run.
              </p>
            ) : null}
          </>
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

function StepRow({
  attemptId,
  onSelect,
  selected,
  step,
  stepLogs,
  wrap,
}: {
  attemptId: string
  onSelect: () => void
  selected: boolean
  step: RepositoryRunStepResponse
  stepLogs: StepLogs
  wrap: boolean
}) {
  const panelId = `run-step-${attemptId}-${step.index}`
  return (
    <div className="border-b border-border">
      <button
        aria-controls={panelId}
        aria-expanded={selected}
        className={`${RUN_STEP_ROW_CLASS} sticky top-0 z-10 bg-background text-left outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring`}
        onClick={onSelect}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn('size-3.5 text-muted-foreground transition-transform', selected && 'rotate-90')}
        />
        <RunStatusIcon state={step.state} />
        <span className="truncate text-sm font-medium">{step.name}</span>
        <span className="flex items-center gap-2.5 text-xs text-muted-foreground">
          {step.exit_code !== null && step.exit_code !== 0 ? (
            <span className="font-mono text-danger-strong">exit {step.exit_code}</span>
          ) : null}
          {step.state === 'skipped'
            ? 'Skipped'
            : <RunDuration end={step.completed_at_unix} start={step.started_at_unix} />}
        </span>
      </button>
      {selected ? (
        <RunLogView
          id={panelId}
          key={panelId}
          logs={stepLogs}
          step={step}
          wrap={wrap}
        />
      ) : null}
    </div>
  )
}
