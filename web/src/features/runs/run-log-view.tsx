import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Check, Copy, TerminalSquare, WrapText } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { StepLogs, StepLogState } from './repository-run-detail-controller'
import type { RepositoryRunStepResponse } from '@/api/types.generated'

const FOLLOW_THRESHOLD_PX = 32
const COPY_CONFIRMATION_MS = 1_500

/**
 * A single step's output: follows new lines while the step runs, wraps or
 * scrolls horizontally on request, and copies the buffered text to the
 * clipboard. The output grows at full length inside whatever scrolls the page,
 * so "following" means keeping the end of this output in view. Callers key
 * this by step so selecting a different step starts following again from a
 * clean state.
 */
export function RunLogView({
  id,
  logs,
  step,
}: {
  id: string
  logs: StepLogs
  step: RepositoryRunStepResponse
}) {
  const logState = logs.state
  const [wrap, setWrap] = useState(true)
  const [following, setFollowing] = useState(true)
  const [copied, setCopied] = useState(false)
  const sectionRef = useRef<HTMLElement>(null)
  const endRef = useRef<HTMLDivElement>(null)
  const isRunning = step.state === 'running'

  useEffect(() => {
    const end = endRef.current
    if (!end) return
    const observer = new IntersectionObserver(
      ([entry]) => setFollowing(entry.isIntersecting),
      { rootMargin: `0px 0px ${FOLLOW_THRESHOLD_PX}px 0px` },
    )
    observer.observe(end)
    return () => observer.disconnect()
  }, [])

  // Layout effects so the scroll lands before paint, and before the observer
  // reports the end of the output as out of view.
  useLayoutEffect(() => {
    if (!logState.viewingEarlier) return
    sectionRef.current?.scrollIntoView({ block: 'start' })
  }, [logState.logs, logState.viewingEarlier])

  useLayoutEffect(() => {
    if (logState.viewingEarlier || !following) return
    endRef.current?.scrollIntoView({ block: 'end' })
  }, [following, logState.logs, logState.viewingEarlier])

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), COPY_CONFIRMATION_MS)
    return () => clearTimeout(timer)
  }, [copied])

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(
        logState.logs.map((log) => log.text).join(''),
      )
      setCopied(true)
    } catch {
      // The browser denied clipboard access; there is nothing to recover.
    }
  }

  return (
    <section
      aria-label={`${step.name} output`}
      className="scroll-mt-14 border-t border-border bg-background text-foreground"
      id={id}
      ref={sectionRef}
    >
      <div className="flex min-h-10 flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-2">
          <TerminalSquare className="size-3.5" />
          {step.name}
          {step.exit_code !== null ? ` · exit ${step.exit_code}` : ''}
        </span>
        <span className="flex items-center gap-3">
          <span>{logStatusLabel(logState, isRunning, following)}</span>
          {logState.logsTruncated ? <span>Some output omitted</span> : null}
          <Button
            aria-pressed={wrap}
            onClick={() => setWrap((value) => !value)}
            size="icon-xs"
            title={wrap ? 'Disable line wrap' : 'Wrap long lines'}
            variant="ghost"
          >
            <WrapText />
          </Button>
          <Button
            disabled={logState.logs.length === 0}
            onClick={() => void handleCopy()}
            size="icon-xs"
            title="Copy output"
            variant="ghost"
          >
            {copied ? <Check /> : <Copy />}
          </Button>
        </span>
      </div>
      {logState.hasEarlier || logState.viewingEarlier ? (
        <div className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-2 text-xs text-muted-foreground">
          {logState.hasEarlier ? (
            <Button disabled={logState.loading} onClick={logs.earlier} size="sm" variant="ghost">
              Load earlier
            </Button>
          ) : null}
          {logState.viewingEarlier ? (
            <>
              <span>Earlier output · live updates paused</span>
              <Button
                disabled={logState.loading}
                onClick={() => {
                  setFollowing(true)
                  logs.latest()
                }}
                size="sm"
                variant="ghost"
              >
                Back to latest
              </Button>
            </>
          ) : <span>Showing recent output</span>}
        </div>
      ) : null}
      {logState.error ? (
        <div
          className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3 text-sm text-danger-strong"
          role="alert"
        >
          <span>{logState.error}</span>
          <Button onClick={logs.retry} size="sm" variant="secondary">
            Retry logs
          </Button>
        </div>
      ) : null}
      <pre
        className={cn(
          'overflow-x-auto break-words px-4 py-4 font-mono text-xs leading-5',
          wrap ? 'whitespace-pre-wrap' : 'whitespace-pre',
        )}
      >
        {logState.logs.length === 0
          ? <span className="text-muted-foreground">No output yet.</span>
          : logState.logs.map((log) => log.text).join('')}
      </pre>
      <div aria-hidden="true" ref={endRef} />
    </section>
  )
}

function logStatusLabel(
  logState: StepLogState,
  isRunning: boolean,
  following: boolean,
) {
  if (logState.loading) return 'Loading output…'
  if (logState.viewingEarlier) return 'Earlier output'
  if (isRunning) {
    return following ? 'Following live output' : 'Paused, scroll down to follow'
  }
  return 'Output finished'
}
