import { Popover } from '@/components/ui/popover'
import { ChevronDown } from 'lucide-react'
import type { ReactNode } from 'react'
import { RunDuration } from './run-duration'
import { runJobShortName } from './run-job-tree-model'
import { RunStatusIcon } from './run-status-icon'
import { runStatus } from './run-status'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunJobDetailResponse,
  RepositoryRunTerminalReason,
} from '@/api/types.generated'

export function RunJobHeader({
  attempt,
  environmentControl,
  jobDetail,
  onSelectAttempt,
  tools,
}: {
  attempt: RepositoryRunAttemptResponse | null
  environmentControl: ReactNode
  jobDetail: RepositoryRunJobDetailResponse
  onSelectAttempt: (attemptId: string) => void
  tools: ReactNode
}) {
  const { attempts, job } = jobDetail
  const ending = attempt ? attemptEnding(attempt) : null
  return (
    <RunJobHeading
      badge={attempt && attempts.length > 1 ? (
        <AttemptMenu attempts={attempts} onSelect={onSelectAttempt} selected={attempt} />
      ) : null}
      detail={attempt ? (
        <>
          {ending}
          {ending ? ' · ' : null}
          <RunDuration end={attempt.completed_at_unix} start={attempt.started_at_unix} />
        </>
      ) : null}
      name={job.key}
      state={attempt?.state ?? job.state}
      terminalReason={attempt?.terminal_reason}
      tools={<>{environmentControl}{tools}</>}
    />
  )
}

export function RunJobHeading({
  badge,
  detail,
  name,
  state,
  terminalReason,
  tools,
}: {
  badge?: ReactNode
  detail: ReactNode
  name: string
  state: string
  terminalReason?: RepositoryRunTerminalReason | null
  tools: ReactNode
}) {
  return (
    <div className="flex min-h-12 flex-none flex-wrap items-center gap-x-2.5 gap-y-1.5 border-b border-border py-2 pl-4 pr-3">
      <span className="flex min-w-0 items-center gap-2.5 max-lg:sr-only">
        <RunStatusIcon state={state} terminalReason={terminalReason} />
        <h2 className="min-w-0 break-words text-[15px] font-semibold leading-5" title={name}>{runJobShortName(name)}</h2>
      </span>
      {badge}
      {detail ? <span className="min-w-0 text-xs text-muted-foreground">{detail}</span> : null}
      {tools ? <div className="ml-auto flex min-w-0 items-center gap-1 max-sm:w-full">{tools}</div> : null}
    </div>
  )
}

function attemptEnding(attempt: RepositoryRunAttemptResponse) {
  const reason = attempt.terminal_reason
  const status = runStatus(attempt.state, reason)
  if (reason && status.label !== attempt.state) {
    return (
      <span className="text-danger-strong">
        {status.label}
        {reason.kind === 'runtime-setup-failed' ? `: ${reason.message}` : null}
      </span>
    )
  }
  const failedStep = attempt.state === 'failed'
    ? attempt.steps.find((step) => step.state === 'failed')
    : undefined
  if (!failedStep) return null
  return (
    <>
      <span className="font-medium text-danger-strong">failed</span> at {failedStep.name}
    </>
  )
}

function AttemptMenu({
  attempts,
  onSelect,
  selected,
}: {
  attempts: readonly RepositoryRunAttemptResponse[]
  onSelect: (attemptId: string) => void
  selected: RepositoryRunAttemptResponse
}) {
  return (
    <Popover
      align="start"
      className="w-64 p-1"
      label="Attempts"
      panel={(close) => (
        <ul className="max-h-[min(20rem,60vh)] overflow-y-auto">
          {attempts.map((attempt) => (
            <li key={attempt.id}>
              <button
                aria-current={attempt.id === selected.id}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring aria-[current=true]:font-medium"
                onClick={() => {
                  onSelect(attempt.id)
                  close()
                }}
                type="button"
              >
                <RunStatusIcon state={attempt.state} terminalReason={attempt.terminal_reason} />
                Attempt {attempt.number}
                <span className="ml-auto text-muted-foreground">
                  {runStatus(attempt.state, attempt.terminal_reason).label}
                  {' · '}
                  <RunDuration end={attempt.completed_at_unix} start={attempt.started_at_unix} />
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      trigger={(props) => (
        <button
          className="flex h-6 items-center gap-1 rounded-md border border-border px-2 text-xs font-medium text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring aria-expanded:bg-muted aria-expanded:text-foreground"
          type="button"
          {...props}
        >
          Attempt {selected.number} of {attempts.length}
          <ChevronDown aria-hidden="true" className="size-3.5" />
        </button>
      )}
    />
  )
}
