import { dropLeading, readAnsi, type LogSpan } from './run-log-ansi'

export type LogLineKind = 'output' | 'command' | 'error' | 'warning' | 'group'

export type LogLine = {
  number: number
  text: string
  spans: LogSpan[] | null
  kind: LogLineKind
  group: number | null
  groupSize: number
  time: string | null
  step: number
}

export type LogStepOf = (second: number | null, previous: number) => number

const MARKER = /^##\[(group|endgroup|error|warning|command)\]/
const TIMESTAMP = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z) ?/

export function readLogLines(text: string, stepOf: LogStepOf | null = null, firstStep = 0): LogLine[] {
  const raw = text.split('\n')
  if (raw.at(-1) === '') raw.pop()
  if (raw[0]?.startsWith('﻿')) raw[0] = raw[0].slice(1)
  const seconds = new Map<string, number>()
  const lines: LogLine[] = []
  let header: LogLine | null = null
  let step = firstStep
  raw.forEach((source, index) => {
    let content = source.endsWith('\r') ? source.slice(0, -1) : source
    let time: string | null = null
    if (stepOf) {
      const stamp = TIMESTAMP.exec(content)
      let second: number | null = null
      if (stamp) {
        time = stamp[1]!
        const whole = time.slice(0, 19)
        second = seconds.get(whole) ?? Date.parse(`${whole}Z`) / 1_000
        seconds.set(whole, second)
        content = content.slice(stamp[0].length)
      }
      step = stepOf(second, step)
    }
    if (header && header.step !== step) header = null
    const ansi = readAnsi(content)
    const marker = ansi.text.startsWith('##[') ? MARKER.exec(ansi.text) : null
    const kind = (marker?.[1] ?? 'output') as LogLineKind | 'endgroup'
    if (kind === 'endgroup') {
      header = null
      return
    }
    const length = marker?.[0].length ?? 0
    const line: LogLine = {
      number: index + 1,
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

export function lineByNumber(lines: readonly LogLine[], number: number) {
  let low = 0
  let high = lines.length - 1
  while (low <= high) {
    const middle = (low + high) >> 1
    const line = lines[middle]!
    if (line.number === number) return line
    if (line.number < number) low = middle + 1
    else high = middle - 1
  }
  return null
}

export function visibleLogLines(lines: readonly LogLine[], openGroups: ReadonlySet<number>): LogLine[] {
  return lines.filter((line) => line.group === null || openGroups.has(line.group))
}

export function logLinesText(lines: readonly LogLine[]) {
  return lines.map((line) => line.text).join('\n')
}
