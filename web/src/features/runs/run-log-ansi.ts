export type AnsiTone = 'black' | 'red' | 'green' | 'yellow' | 'blue' | 'magenta' | 'cyan' | 'white'

export type LogStyle = {
  tone: AnsiTone | null
  bright: boolean
  bold: boolean
  dim: boolean
}

export type LogSpan = { text: string; style: LogStyle }

export type AnsiText = { text: string; spans: LogSpan[] | null; style: LogStyle }

const ESC = '\u001b'
const PLAIN: LogStyle = { tone: null, bright: false, bold: false, dim: false }
const TONES: AnsiTone[] = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
const PALETTE: readonly (readonly [number, number, number])[] = [
  [0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0], [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229],
  [127, 127, 127], [255, 0, 0], [0, 255, 0], [255, 255, 0], [92, 92, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255],
]

export function readAnsi(raw: string, carried: LogStyle = PLAIN): AnsiText {
  if (!raw.includes(ESC) && sameStyle(carried, PLAIN)) return { text: raw, spans: null, style: carried }
  const spans: LogSpan[] = []
  let style = carried
  let pending = ''
  let index = 0
  const flush = () => {
    if (!pending) return
    const last = spans.at(-1)
    if (last && sameStyle(last.style, style)) last.text += pending
    else spans.push({ text: pending, style })
    pending = ''
  }
  while (index < raw.length) {
    const escape = raw.indexOf(ESC, index)
    if (escape === -1) {
      pending += raw.slice(index)
      break
    }
    pending += raw.slice(index, escape)
    const kind = raw[escape + 1]
    if (kind === '[') {
      let end = escape + 2
      while (end < raw.length && !isFinalByte(raw.charCodeAt(end))) end++
      if (raw[end] === 'm') {
        flush()
        style = applySgr(style, raw.slice(escape + 2, end))
      }
      index = end + 1
    } else if (kind === ']') {
      index = oscEnd(raw, escape + 2)
    } else if (kind !== undefined && '()*+'.includes(kind)) {
      index = escape + 3
    } else {
      index = escape + 2
    }
  }
  flush()
  const text = spans.map((span) => span.text).join('')
  return { text, spans: spans.some((span) => !sameStyle(span.style, PLAIN)) ? spans : null, style }
}

export function dropLeading(spans: readonly LogSpan[], count: number): LogSpan[] {
  const kept: LogSpan[] = []
  let remaining = count
  for (const span of spans) {
    if (remaining >= span.text.length) {
      remaining -= span.text.length
      continue
    }
    kept.push(remaining > 0 ? { text: span.text.slice(remaining), style: span.style } : span)
    remaining = 0
  }
  return kept
}

function isFinalByte(code: number) {
  return code >= 0x40 && code <= 0x7e
}

function oscEnd(raw: string, start: number) {
  for (let index = start; index < raw.length; index++) {
    if (raw[index] === '\u0007') return index + 1
    if (raw[index] === ESC && raw[index + 1] === '\\') return index + 2
  }
  return raw.length
}

function sameStyle(left: LogStyle, right: LogStyle) {
  return left.tone === right.tone && left.bright === right.bright && left.bold === right.bold && left.dim === right.dim
}

function applySgr(current: LogStyle, params: string): LogStyle {
  const codes = params === '' ? [0] : params.split(/[;:]/).map((part) => Number(part) || 0)
  let style = current
  for (let index = 0; index < codes.length; index++) {
    const code = codes[index]
    if (code === undefined) break
    if (code === 0) style = PLAIN
    else if (code === 1) style = { ...style, bold: true }
    else if (code === 2) style = { ...style, dim: true }
    else if (code === 22) style = { ...style, bold: false, dim: false }
    else if (code >= 30 && code <= 37) style = withPaletteColor(style, code - 30)
    else if (code >= 90 && code <= 97) style = withPaletteColor(style, code - 90 + 8)
    else if (code === 39) style = { ...style, tone: null, bright: false }
    else if (code === 38 || code === 48) {
      const extended = readExtendedColor(codes, index)
      index = extended.next
      if (code === 38 && extended.color !== null) style = withPaletteColor(style, extended.color)
    }
  }
  return style
}

function readExtendedColor(codes: readonly number[], index: number): { color: number | null; next: number } {
  if (codes[index + 1] === 5) {
    const value = codes[index + 2]
    return { color: value === undefined ? null : paletteFor256(value), next: index + 2 }
  }
  if (codes[index + 1] === 2) {
    const [red, green, blue] = [codes[index + 2], codes[index + 3], codes[index + 4]]
    if (red === undefined || green === undefined || blue === undefined) return { color: null, next: codes.length }
    return { color: nearestPalette(red, green, blue), next: index + 4 }
  }
  return { color: null, next: index }
}

function paletteFor256(value: number) {
  if (value < 16) return value
  if (value < 232) {
    const cube = value - 16
    return nearestPalette(
      cubeLevel(Math.floor(cube / 36)),
      cubeLevel(Math.floor(cube / 6) % 6),
      cubeLevel(cube % 6),
    )
  }
  const level = 8 + (Math.min(value, 255) - 232) * 10
  return nearestPalette(level, level, level)
}

function cubeLevel(step: number) {
  return step === 0 ? 0 : 55 + step * 40
}

function nearestPalette(red: number, green: number, blue: number) {
  let best = 0
  let bestDistance = Infinity
  PALETTE.forEach(([r, g, b], index) => {
    const distance = (r - red) ** 2 + (g - green) ** 2 + (b - blue) ** 2
    if (distance < bestDistance) {
      best = index
      bestDistance = distance
    }
  })
  return best
}

function withPaletteColor(style: LogStyle, color: number): LogStyle {
  return { ...style, tone: TONES[color % 8] ?? null, bright: color >= 8 }
}
