import type { LogSpan, LogStyle } from './run-log-ansi'
import type { LogLine } from './run-log-lines'

export type LogMatch = { line: number; start: number; end: number }

export type LogPiece = { text: string; style: LogStyle | null; match: 'current' | 'other' | null }

export function findLogMatches(lines: readonly LogLine[], query: string): LogMatch[] {
  if (!query) return []
  const pattern = new RegExp(query.replace(/[\\^$.*+?()[\]{}|/]/g, '\\$&'), 'giu')
  const matches: LogMatch[] = []
  for (const line of lines) {
    for (const found of line.text.matchAll(pattern)) {
      matches.push({ line: line.number, start: found.index, end: found.index + found[0].length })
    }
  }
  return matches
}

export function logLinePieces(
  line: Pick<LogLine, 'spans' | 'text'>,
  matches: readonly LogMatch[],
  current: LogMatch | null,
): LogPiece[] {
  const spans: readonly (LogSpan | { text: string; style: null })[] = line.spans ?? [{ text: line.text, style: null }]
  if (matches.length === 0) return spans.map((span) => ({ ...span, match: null }))
  const pieces: LogPiece[] = []
  let offset = 0
  for (const span of spans) {
    const spanEnd = offset + span.text.length
    let cursor = offset
    for (const match of matches) {
      if (match.end <= cursor || match.start >= spanEnd) continue
      const start = Math.max(match.start, cursor)
      const end = Math.min(match.end, spanEnd)
      if (start > cursor) pieces.push({ text: span.text.slice(cursor - offset, start - offset), style: span.style, match: null })
      pieces.push({ text: span.text.slice(start - offset, end - offset), style: span.style, match: match === current ? 'current' : 'other' })
      cursor = end
    }
    if (cursor < spanEnd) pieces.push({ text: span.text.slice(cursor - offset), style: span.style, match: null })
    offset = spanEnd
  }
  return pieces
}
