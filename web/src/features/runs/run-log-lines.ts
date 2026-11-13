import { dropLeading, readAnsi, type LogSpan, type LogStyle } from './run-log-ansi'

export type LogLineKind = 'output' | 'command' | 'error' | 'warning' | 'group'

export type LogLine = {
  number: number
  anchor: string
  text: string
  spans: LogSpan[] | null
  kind: LogLineKind
  group: number | null
  groupSize: number
  time: string | null
  step: number
}

export type LogStamp = { second: number | null; group: boolean }

const MARKER = /^##\[(group|endgroup|error|warning|command)\]/
const TIMESTAMP = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z) ?/

export function readLogLines(text: string, stepsOf?: (stamps: readonly LogStamp[]) => readonly number[]): LogLine[] {
  const raw = text.split('\n')
  if (raw.at(-1) === '') raw.pop()
  if (raw[0]?.startsWith('\uFEFF')) raw[0] = raw[0].slice(1)
  const seconds = new Map<string, number>()
  const sources = raw.map((source) => {
    const content = source.endsWith('\r') ? source.slice(0, -1) : source
    return stepsOf ? stampedSource(content, seconds) : { content, time: null, second: null }
  })
  const steps = stepsOf
    ? stepsOf(sources.map(({ content, second }) => ({ second, group: content.startsWith('##[group]') })))
    : null
  const lines: LogLine[] = []
  let header: LogLine | null = null
  let style: LogStyle | undefined
  sources.forEach(({ content, time }, index) => {
    const step = steps?.[index] ?? 0
    if (header && header.step !== step) header = null
    const ansi = readAnsi(content, style)
    style = ansi.style
    const marker = ansi.text.startsWith('##[') ? MARKER.exec(ansi.text) : null
    const kind = (marker?.[1] ?? 'output') as LogLineKind | 'endgroup'
    if (kind === 'endgroup') {
      header = null
      return
    }
    const length = marker?.[0].length ?? 0
    const line: LogLine = {
      number: index + 1,
      anchor: String(index + 1),
      text: length ? ansi.text.slice(length) : ansi.text,
      spans: length && ansi.spans ? dropLeading(ansi.spans, length) : ansi.spans,
      kind,
      group: kind === 'group' ? null : header?.number ?? null,
      groupSize: 0,
      time,
      step,
    }
    if (kind === 'group') header = line
    else if (header) header.groupSize++
    lines.push(line)
  })
  return lines
}

function stampedSource(content: string, seconds: Map<string, number>) {
  const stamp = TIMESTAMP.exec(content)
  const time = stamp?.[1]
  if (!stamp || time === undefined) return { content, time: null, second: null }
  const whole = time.slice(0, 19)
  const second = seconds.get(whole) ?? Date.parse(`${whole}Z`) / 1_000
  seconds.set(whole, second)
  return { content: content.slice(stamp[0].length), time, second }
}

export function lineByNumber(lines: readonly LogLine[], number: number) {
  let low = 0
  let high = lines.length - 1
  while (low <= high) {
    const middle = (low + high) >> 1
    const line = lines[middle]
    if (line === undefined) return null
    if (line.number === number) return line
    if (line.number < number) low = middle + 1
    else high = middle - 1
  }
  return null
}

export function lineByAnchor(lines: readonly LogLine[], anchor: string | null) {
  return anchor === null ? null : lines.find((line) => line.anchor === anchor) ?? null
}

export function visibleLogLines(lines: readonly LogLine[], openGroups: ReadonlySet<number>): LogLine[] {
  return lines.filter((line) => line.group === null || openGroups.has(line.group))
}

export function logLinesText(lines: readonly LogLine[]) {
  return lines.map((line) => line.text).join('\n')
}
