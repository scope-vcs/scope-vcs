import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { ArrowDown } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { StepLogs } from './repository-run-detail-controller'
import type { RepositoryRunStepResponse } from '@/api/types.generated'

const FOLLOW_THRESHOLD_PX = 32

/**
 * A single step's output, led by the command that produced it. The output
 * grows at full length inside whatever scrolls the page, so "following" means
 * keeping the end of this output in view. While a running step's end is out of
 * view, a button counts the lines that arrived since. Callers key this by step
 * so selecting a different step starts following again from a clean state.
 */
export function RunLogView({
  id,
  logs,
  step,
  wrap,
}: {
  id: string
  logs: StepLogs
  step: RepositoryRunStepResponse
  wrap: boolean
}) {
  const logState = logs.state
  const text = logState.logs.map((log) => log.text).join('')
  const [following, setFollowing] = useState(true)
  // The log position the output had reached when the reader scrolled away.
  // Positions stay stable while the cached window drops its oldest chunks.
  const [pausedAt, setPausedAt] = useState<number | null>(null)
  const lastPosition = logState.logs.at(-1)?.position ?? -1
  const lastPositionRef = useRef(lastPosition)
  const sectionRef = useRef<HTMLElement>(null)
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    lastPositionRef.current = lastPosition
  }, [lastPosition])

  useEffect(() => {
    const end = endRef.current
    if (!end) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        setFollowing(entry.isIntersecting)
        setPausedAt(entry.isIntersecting ? null : lastPositionRef.current)
      },
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

  const newLines = step.state === 'running' && pausedAt !== null
    ? countLinesAfter(logState.logs, pausedAt)
    : 0

  return (
    <section
      aria-label={`${step.name} output`}
      className="scroll-mt-9 bg-background pl-4 pr-4 pt-2 font-mono text-xs leading-5 text-foreground sm:pl-[3.75rem]"
      id={id}
      ref={sectionRef}
    >
      <p className="whitespace-pre-wrap break-words text-muted-foreground">$ {step.command}</p>
      {logState.hasEarlier ? (
        <button
          className="mt-1 font-sans text-muted-foreground underline underline-offset-2 hover:text-foreground disabled:opacity-50"
          disabled={logState.loading}
          onClick={logs.earlier}
          type="button"
        >
          Load earlier output
        </button>
      ) : null}
      {logState.logsTruncated ? (
        <p className="mt-1 font-sans text-muted-foreground">Some output was omitted.</p>
      ) : null}
      {logState.error ? (
        <p className="mt-1 flex flex-wrap items-center gap-3 font-sans text-danger-strong" role="alert">
          {logState.error}
          <Button onClick={logs.retry} size="sm" variant="secondary">
            Retry
          </Button>
        </p>
      ) : null}
      <pre
        className={cn(
          'mt-1 overflow-x-auto break-words pb-4',
          wrap ? 'whitespace-pre-wrap' : 'whitespace-pre',
        )}
      >
        {text.length > 0
          ? text
          : <span className="text-muted-foreground">{logState.loading ? 'Loading output…' : 'No output yet.'}</span>}
      </pre>
      <div aria-hidden="true" ref={endRef} />
      {logState.viewingEarlier || newLines > 0 ? (
        // A zero-height sticky row whose button grows upward from its bottom
        // edge, so it floats over the output without adding to its length.
        <div className="sticky bottom-4 z-10 flex h-0 items-end justify-center">
          <Button
            className="rounded-full font-sans shadow-[var(--shadow-pop)]"
            disabled={logState.loading && logState.viewingEarlier}
            onClick={() => {
              if (logState.viewingEarlier) logs.latest()
              setFollowing(true)
            }}
            size="sm"
          >
            <ArrowDown />
            {logState.viewingEarlier
              ? 'Back to latest'
              : `${newLines} new ${newLines === 1 ? 'line' : 'lines'}`}
          </Button>
        </div>
      ) : null}
    </section>
  )
}

function countLinesAfter(
  logs: readonly { position: number; text: string }[],
  position: number,
) {
  let count = 0
  for (const log of logs) {
    if (log.position <= position) continue
    for (let index = log.text.indexOf('\n'); index !== -1; index = log.text.indexOf('\n', index + 1)) {
      count += 1
    }
  }
  return count
}
