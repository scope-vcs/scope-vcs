import { cn } from '@/lib/utils'
import { Search } from 'lucide-react'
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import type { LogLine } from './run-log-lines'
import { findLogMatches, type LogMatch } from './run-log-search'
import type { RunLogFocus } from './run-log-text'

const COPY_CONFIRMATION_MS = 1_500

export type RunLogSearch = ReturnType<typeof useRunLogSearch>

export function useRunLogSearch(lines: readonly LogLine[]) {
  const [query, setQuery] = useState('')
  const [index, setIndex] = useState(0)
  const [focused, setFocused] = useState(true)
  const matches = useMemo(() => findLogMatches(lines, query), [lines, query])
  const byLine = useMemo(() => {
    const grouped = new Map<number, LogMatch[]>()
    for (const match of matches) {
      const listed = grouped.get(match.line)
      if (listed) listed.push(match)
      else grouped.set(match.line, [match])
    }
    return grouped
  }, [matches])
  const position = matches.length === 0 ? -1 : Math.min(index, matches.length - 1)
  const current = position === -1 ? null : matches[position]!
  const currentLine = focused ? current?.line ?? null : null
  const focus = useMemo<RunLogFocus | null>(
    () => (currentLine === null ? null : { line: currentLine, match: position }),
    [currentLine, position],
  )

  return {
    byLine,
    count: matches.length,
    current,
    focus,
    position,
    query,
    move(delta: 1 | -1) {
      if (matches.length === 0) return
      setIndex((position + delta + matches.length) % matches.length)
      setFocused(true)
    },
    release() {
      setFocused(false)
    },
    search(value: string) {
      setQuery(value)
      setIndex(0)
      setFocused(true)
    },
  }
}

export function RunLogTools({
  copyText,
  search,
  times,
  onToggleTimes,
  onToggleWrap,
  wrap,
}: {
  copyText: string
  search: RunLogSearch
  times?: boolean
  onToggleTimes?: () => void
  onToggleWrap: () => void
  wrap: boolean
}) {
  return (
    <div className="flex min-w-0 items-center gap-1 max-sm:w-full">
      <RunLogSearchBox search={search} />
      <ToolToggle onClick={onToggleWrap} pressed={wrap} title={wrap ? 'Disable line wrap' : 'Wrap long lines'}>Wrap</ToolToggle>
      {onToggleTimes ? (
        <ToolToggle onClick={onToggleTimes} pressed={times ?? false} title={times ? 'Hide timestamps' : 'Show timestamps'}>Times</ToolToggle>
      ) : null}
      <CopyButton text={copyText} />
    </div>
  )
}

function RunLogSearchBox({ search }: { search: RunLogSearch }) {
  return (
    <label className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-md border border-border bg-card px-2 text-xs focus-within:border-ring focus-within:outline focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-ring sm:w-52 sm:flex-none">
      <Search aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
      <input
        aria-label="Search log"
        className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
        onChange={(event) => search.search(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== 'Enter') return
          event.preventDefault()
          search.move(event.shiftKey ? -1 : 1)
        }}
        placeholder="Search log"
        type="search"
        value={search.query}
      />
      {search.query ? (
        <span aria-live="polite" className="shrink-0 tabular-nums text-muted-foreground">
          {search.count === 0 ? 'No matches' : `${search.position + 1} of ${search.count}`}
        </span>
      ) : null}
    </label>
  )
}

function ToolToggle({
  children,
  onClick,
  pressed,
  title,
}: {
  children: ReactNode
  onClick: () => void
  pressed: boolean
  title: string
}) {
  return (
    <button
      aria-pressed={pressed}
      className={TOOL_BUTTON_CLASS}
      onClick={onClick}
      title={title}
      type="button"
    >
      {children}
    </button>
  )
}

const TOOL_BUTTON_CLASS =
  'inline-flex h-7 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-45 aria-pressed:bg-muted aria-pressed:text-foreground'

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
    <button
      className={cn(TOOL_BUTTON_CLASS, copied && 'text-success-strong hover:text-success-strong')}
      disabled={text.length === 0}
      onClick={() => void handleCopy()}
      title="Copy output"
      type="button"
    >
      {copied ? 'Copied' : 'Copy'}
    </button>
  )
}
