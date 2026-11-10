import { cn } from '@/lib/utils'
import { clsx } from 'clsx'
import { ChevronRight } from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import type { AnsiTone, LogStyle } from './run-log-ansi'
import { logBlocks } from './run-log-blocks'
import { lineByNumber, visibleLogLines, type LogLine } from './run-log-lines'
import { logLinePieces, type LogMatch } from './run-log-search'

const LOG_LINE_HEIGHT_PX = 20
const NO_MATCHES: readonly LogMatch[] = []

const TONE_CLASS: Record<AnsiTone, string> = {
  black: 'text-muted-foreground',
  red: 'text-danger-strong',
  green: 'text-success-strong',
  yellow: 'text-warning-strong',
  blue: 'text-info-strong',
  magenta: 'text-log-magenta',
  cyan: 'text-log-cyan',
  white: 'text-foreground',
}

const KIND_ROW_CLASS: Record<LogLine['kind'], string> = {
  output: '',
  command: 'text-info-strong',
  error: 'bg-danger-soft text-danger-strong shadow-[inset_2px_0_0_var(--color-danger)]',
  warning: 'bg-warning-soft text-warning-strong shadow-[inset_2px_0_0_var(--color-warning)]',
  group: 'text-muted-foreground',
}

export type RunLogFocus = { line: number; match?: number }

type RunLogTextProps = {
  currentMatch: LogMatch | null
  empty: ReactNode
  focus: RunLogFocus | null
  followTail: boolean
  lineId: (line: number) => string
  lines: readonly LogLine[]
  markedLine: number | null
  matches: ReadonlyMap<number, readonly LogMatch[]>
  onLinkLine: (line: number) => void
  times: boolean
  wrap: boolean
}

export function RunLogText({
  currentMatch,
  empty,
  focus,
  followTail,
  lineId,
  lines,
  markedLine,
  matches,
  onLinkLine,
  times,
  wrap,
}: RunLogTextProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [openGroups, setOpenGroups] = useState<ReadonlySet<number>>(() => new Set())
  const forced = [currentMatch?.line, focus?.line, markedLine]
    .map((number) => (number === undefined || number === null ? null : lineByNumber(lines, number)?.group ?? null))
    .filter((group): group is number => group !== null)
  const forcedKey = forced.join(',')
  const shownGroups = useMemo(
    () => forcedKey ? new Set([...openGroups, ...forcedKey.split(',').map(Number)]) : openGroups,
    [forcedKey, openGroups],
  )
  const blocks = useMemo(() => logBlocks(visibleLogLines(lines, shownGroups)), [lines, shownGroups])
  const eagerLines = [currentMatch?.line, focus?.line, markedLine]
  const focusHere = focus !== null && lineByNumber(lines, focus.line) !== null
  const gutter = String(lines.at(-1)?.number ?? 1).length + 3

  useLayoutEffect(() => {
    if (!focus) return
    const line = containerRef.current?.querySelector<HTMLElement>(`[data-line="${focus.line}"]`)
    if (line) centerInScroller(line)
  }, [focus])

  function toggleGroup(group: number) {
    setOpenGroups((current) => {
      const next = new Set(current)
      if (!next.delete(group)) next.add(group)
      return next
    })
  }

  if (lines.length === 0) {
    return <p className="px-4 py-1 font-sans text-xs text-muted-foreground">{empty}</p>
  }
  return (
    <div
      className={cn('overflow-x-auto py-1 font-mono text-xs leading-5 text-foreground', !wrap && 'whitespace-pre')}
      ref={containerRef}
      style={{ '--log-gutter': `${gutter}ch` } as CSSProperties}
    >
      {blocks.map((block, index) => (
        <LogBlock
          currentMatch={currentMatch}
          eager={(index === 0 && !focusHere) || (followTail && index === blocks.length - 1) || block.some((line) => eagerLines.includes(line.number))}
          key={index}
          lineId={lineId}
          lines={block}
          markedLine={markedLine}
          matches={matches}
          onLinkLine={onLinkLine}
          onToggleGroup={toggleGroup}
          openGroups={shownGroups}
          times={times}
          wrap={wrap}
        />
      ))}
    </div>
  )
}

type LogBlockProps = {
  currentMatch: LogMatch | null
  eager: boolean
  lineId: (line: number) => string
  lines: readonly LogLine[]
  markedLine: number | null
  matches: ReadonlyMap<number, readonly LogMatch[]>
  onLinkLine: (line: number) => void
  onToggleGroup: (group: number) => void
  openGroups: ReadonlySet<number>
  times: boolean
  wrap: boolean
}

function LogBlock({ eager, lines, wrap, ...rowProps }: LogBlockProps) {
  const blockRef = useRef<HTMLDivElement>(null)
  const [reached, setReached] = useState(eager)
  if (eager && !reached) setReached(true)
  const rendered = eager || reached

  useEffect(() => {
    if (rendered) return
    const block = blockRef.current
    if (!block) return
    if (!('oncontentvisibilityautostatechange' in block)) {
      setReached(true)
      return
    }
    const onState = (event: Event) => {
      if (!(event as Event & { skipped: boolean }).skipped) setReached(true)
    }
    block.addEventListener('contentvisibilityautostatechange', onState)
    return () => block.removeEventListener('contentvisibilityautostatechange', onState)
  }, [rendered])

  return (
    <div
      className={cn('[content-visibility:auto]', !wrap && 'w-max min-w-full')}
      data-log-block=""
      ref={blockRef}
      style={{ containIntrinsicBlockSize: `auto ${lines.length * LOG_LINE_HEIGHT_PX}px` }}
    >
      {rendered ? (
        <div>
          {lines.map((line) => <LogRow key={line.number} line={line} wrap={wrap} {...rowProps} />)}
        </div>
      ) : (
        <div style={{ height: lines.length * LOG_LINE_HEIGHT_PX }} />
      )}
    </div>
  )
}

function LogRow({
  currentMatch,
  line,
  lineId,
  markedLine,
  matches,
  onLinkLine,
  onToggleGroup,
  openGroups,
  times,
  wrap,
}: Omit<LogBlockProps, 'eager' | 'lines'> & { line: LogLine }) {
  const id = lineId(line.number)
  const group = line.kind === 'group'
  const open = group && openGroups.has(line.number)
  const pieces = logLinePieces(line, matches.get(line.number) ?? NO_MATCHES, currentMatch)
  return (
    <div
      className={clsx(
        'grid scroll-mt-12 items-start',
        times ? 'grid-cols-[var(--log-gutter)_1rem_6.5rem_minmax(0,1fr)]' : 'grid-cols-[var(--log-gutter)_1rem_minmax(0,1fr)]',
        KIND_ROW_CLASS[line.kind],
        markedLine === line.number && 'outline outline-1 -outline-offset-1 outline-border-strong',
        markedLine === line.number && (line.kind === 'output' || line.kind === 'command') && 'bg-info-soft',
      )}
      data-line={line.number}
      id={id}
    >
      <a
        className="select-none pr-3 text-right text-muted-foreground/70 outline-none hover:text-foreground focus-visible:text-foreground focus-visible:underline"
        href={`#${id}`}
        onClick={(event) => {
          event.preventDefault()
          onLinkLine(line.number)
        }}
      >
        {line.number}
      </a>
      {group ? (
        <button
          aria-expanded={open}
          aria-label={`${open ? 'Collapse' : 'Expand'} ${line.text}`}
          className="flex h-5 items-center text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
          onClick={() => onToggleGroup(line.number)}
          type="button"
        >
          <ChevronRight aria-hidden="true" className={clsx('size-3 transition-transform', open && 'rotate-90')} />
        </button>
      ) : <span />}
      {times ? (
        <span className="select-none text-muted-foreground/70" title={line.time ?? undefined}>
          {line.time?.slice(11, 19) ?? ''}
        </span>
      ) : null}
      <span
        className={clsx('min-w-0 pr-4', wrap ? 'whitespace-pre-wrap break-words [overflow-wrap:anywhere]' : 'whitespace-pre', group && 'cursor-pointer hover:text-foreground')}
        onClick={group ? () => onToggleGroup(line.number) : undefined}
      >
        {pieces.map((piece, index) => (
          <LogPiece key={index} match={piece.match} style={piece.style} text={piece.text} />
        ))}
        {group && !open ? (
          <span className="ml-2 select-none rounded-full bg-muted px-1.5 py-px font-sans text-[11px] text-muted-foreground">
            {line.groupSize} {line.groupSize === 1 ? 'line' : 'lines'}
          </span>
        ) : null}
      </span>
    </div>
  )
}

function LogPiece({ match, style, text }: { match: 'current' | 'other' | null; style: LogStyle | null; text: string }) {
  const className = style
    ? clsx(style.tone && TONE_CLASS[style.tone], style.bold && 'font-semibold', style.dim && 'opacity-70')
    : undefined
  if (match) {
    return (
      <mark
        className={clsx('rounded-[2px] text-inherit', match === 'current' ? 'bg-log-match-current' : 'bg-log-match', className)}
        data-current-match={match === 'current' ? '' : undefined}
      >
        {text}
      </mark>
    )
  }
  return className ? <span className={className}>{text}</span> : text
}

function centerInScroller(element: HTMLElement) {
  let scroller = element.parentElement
  while (scroller && !(scroller.scrollHeight > scroller.clientHeight && /auto|scroll/.test(getComputedStyle(scroller).overflowY))) {
    scroller = scroller.parentElement
  }
  const box = element.getBoundingClientRect()
  if (!scroller) {
    window.scrollBy({ top: box.top + box.height / 2 - window.innerHeight / 2 })
    return
  }
  const frame = scroller.getBoundingClientRect()
  scroller.scrollTop += box.top + box.height / 2 - (frame.top + frame.height / 2)
}

export function RunLogPlaceholder() {
  return (
    <div aria-busy="true" aria-label="Loading log" className="space-y-2 px-4 py-2.5" role="status">
      {['w-2/5', 'w-3/4', 'w-1/2', 'w-2/3', 'w-1/3'].map((width) => (
        <div className="flex items-center gap-3" key={width}>
          <span className="h-2.5 w-6 rounded-sm bg-muted" />
          <span className={cn('h-2.5 rounded-sm bg-muted', width)} />
        </div>
      ))}
    </div>
  )
}
