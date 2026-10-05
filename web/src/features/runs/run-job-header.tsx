import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { Check, ChevronDown, Copy, WrapText } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import { runStatus } from './run-status'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunJobDetailResponse,
} from '@/api/types.generated'

const COPY_CONFIRMATION_MS = 1_500

export function RunJobHeader({
  attempt,
  environmentControl,
  jobDetail,
  logText,
  onSelectAttempt,
  onToggleWrap,
  wrap,
}: {
  attempt: RepositoryRunAttemptResponse | null
  environmentControl: ReactNode
  jobDetail: RepositoryRunJobDetailResponse
  logText: string | null
  onSelectAttempt: (attemptId: string) => void
  onToggleWrap: () => void
  wrap: boolean
}) {
  const { attempts, job } = jobDetail
  const ending = attempt ? attemptEnding(attempt) : null
  return (
    <div className="flex min-h-12 flex-none flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-border py-2 pl-4 pr-3">
      <RunStatusIcon
        state={attempt?.state ?? job.state}
        terminalReason={attempt?.terminal_reason}
      />
      <h2 className="min-w-0 max-w-full truncate text-[15px] font-semibold">{job.key}</h2>
      {attempt && attempts.length > 1 ? (
        <AttemptMenu
          attempts={attempts}
          onSelect={onSelectAttempt}
          selected={attempt}
        />
      ) : null}
      {ending}
      <span className="ml-auto flex items-center gap-1">
        {attempt ? (
          <span className="mr-1 text-xs text-muted-foreground">
            <RunDuration end={attempt.completed_at_unix} start={attempt.started_at_unix} />
          </span>
        ) : null}
        {environmentControl}
        {logText !== null ? (
          <RunLogControls onToggleWrap={onToggleWrap} text={logText} wrap={wrap} />
        ) : null}
      </span>
    </div>
  )
}

export function RunLogControls({
  onToggleWrap,
  text,
  wrap,
}: {
  onToggleWrap: () => void
  text: string
  wrap: boolean
}) {
  return (
    <>
      <Button
        aria-pressed={wrap}
        onClick={onToggleWrap}
        size="icon-xs"
        title={wrap ? 'Disable line wrap' : 'Wrap long lines'}
        variant="ghost"
      >
        <WrapText />
      </Button>
      <CopyButton text={text} />
    </>
  )
}

function attemptEnding(attempt: RepositoryRunAttemptResponse) {
  const reason = attempt.terminal_reason
  const status = runStatus(attempt.state, reason)
  if (reason && status.label !== attempt.state) {
    return (
      <span className="min-w-0 truncate text-xs text-danger-strong">
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
    <span className="min-w-0 truncate text-xs text-muted-foreground">
      <span className="text-danger-strong">failed</span> at {failedStep.name}
    </span>
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

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), COPY_CONFIRMATION_MS)
    return () => clearTimeout(timer)
  }, [copied])

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
    } catch {}
  }

  return (
    <Button
      className={cn(copied && 'text-success-strong')}
      disabled={text.length === 0}
      onClick={() => void handleCopy()}
      size="icon-xs"
      title="Copy output"
      variant="ghost"
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  )
}
