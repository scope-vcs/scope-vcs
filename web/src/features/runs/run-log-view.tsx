import { Button } from '@/components/ui/button'
import { ArrowDown } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { RepositoryRunStepResponse } from '@/api/types.generated'
import type { StepLogs } from './repository-run-detail-controller'
import type { LogLine } from './run-log-lines'
import { RunLogPlaceholder, RunLogText, type RunLogFocus } from './run-log-text'
import { useRunLogPaintTiming } from './run-log-timing'
import type { RunLogSearch } from './run-log-tools'

const FOLLOW_THRESHOLD_PX = 32

export function RunLogView({
  focus,
  lineId,
  lines,
  logs,
  markedLine,
  onFollowTail,
  onLinkLine,
  runId,
  search,
  step,
  text,
  wrap,
}: {
  focus: RunLogFocus | null
  lineId: (line: LogLine) => string
  lines: readonly LogLine[]
  logs: StepLogs
  markedLine: number | null
  onFollowTail: () => void
  onLinkLine: (line: LogLine) => void
  runId: string
  search: RunLogSearch
  step: RepositoryRunStepResponse
  text: string
  wrap: boolean
}) {
  const logState = logs.state
  useRunLogPaintTiming({
    provider: 'native',
    runId,
    settled: !logState.loading && (logState.initialized || logState.error !== null),
    text,
  })
  const [following, setFollowing] = useState(true)
  const [pausedAt, setPausedAt] = useState<number | null>(null)
  const lastPosition = logState.logs.at(-1)?.position ?? -1
  const lastPositionRef = useRef(lastPosition)
  const startRef = useRef<HTMLDivElement>(null)
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    lastPositionRef.current = lastPosition
  }, [lastPosition])

  useEffect(() => {
    const end = endRef.current
    if (!end) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry) return
        setFollowing(entry.isIntersecting)
        setPausedAt(entry.isIntersecting ? null : lastPositionRef.current)
      },
      { rootMargin: `0px 0px ${FOLLOW_THRESHOLD_PX}px 0px` },
    )
    observer.observe(end)
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    if (!logState.viewingEarlier || focus) return
    startRef.current?.scrollIntoView({ block: 'start' })
  }, [focus, logState.logs, logState.viewingEarlier])

  useLayoutEffect(() => {
    if (logState.viewingEarlier || !following || focus) return
    endRef.current?.scrollIntoView({ block: 'end' })
  }, [focus, following, logState.logs, logState.viewingEarlier])

  useEffect(() => {
    const view = startRef.current
    if (!view || logState.viewingEarlier || !following || focus) return
    const observer = new ResizeObserver(() => endRef.current?.scrollIntoView({ block: 'end' }))
    observer.observe(view)
    return () => observer.disconnect()
  }, [focus, following, logState.viewingEarlier])

  const newLines = step.state === 'running' && pausedAt !== null
    ? countLinesAfter(logState.logs, pausedAt)
    : 0

  return (
    <div className="scroll-mt-9" ref={startRef}>
      <p className="whitespace-pre-wrap break-words px-4 pt-2 font-mono text-xs leading-5 text-muted-foreground">$ {step.command}</p>
      {logState.hasEarlier ? (
        <button
          className="mx-4 mt-1 text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground disabled:opacity-50"
          disabled={logState.loading}
          onClick={logs.earlier}
          type="button"
        >
          Load earlier output
        </button>
      ) : null}
      {logState.logsTruncated ? (
        <p className="px-4 pt-1 text-xs text-muted-foreground">Some output was omitted.</p>
      ) : null}
      {logState.error ? (
        <p className="flex flex-wrap items-center gap-3 px-4 pt-1 text-sm text-danger-strong" role="alert">
          {logState.error}
          <Button onClick={logs.retry} size="sm" variant="secondary">
            Retry
          </Button>
        </p>
      ) : null}
      {lines.length === 0 && logState.loading ? <RunLogPlaceholder /> : (
        <RunLogText
          currentMatch={search.current}
          empty="No output yet."
          focus={focus}
          followTail
          lineId={lineId}
          lines={lines}
          markedLine={markedLine}
          matches={search.byLine}
          onLinkLine={onLinkLine}
          times={false}
          wrap={wrap}
        />
      )}
      <div aria-hidden="true" ref={endRef} />
      {logState.viewingEarlier || newLines > 0 ? (
        <div className="sticky bottom-4 z-10 flex h-0 items-end justify-center">
          <Button
            className="rounded-full shadow-[var(--shadow-pop)]"
            disabled={logState.loading && logState.viewingEarlier}
            onClick={() => {
              if (logState.viewingEarlier) logs.latest()
              onFollowTail()
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
    </div>
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
