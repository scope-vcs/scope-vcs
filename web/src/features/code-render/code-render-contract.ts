import type { ReviewDiffOmittedReason } from '../../api/types'

export const CODE_RENDER_STARTUP_DEADLINE_MS = 10_000

export type CodeRenderDiffInput = {
  budget: {
    maxHunks: number
    maxOutputBytes: number
    maxRenderedLines: number
  }
  kind: 'diff'
  newText: string
  oldText: string
  path: string
}

export type CodeRenderFileInput = {
  budget: { maxOutputBytes: number }
  kind: 'file'
  path: string
  text: string
}

export type CodeRenderWorkerInput = CodeRenderDiffInput | CodeRenderFileInput

export type CodeRenderWorkerResult =
  | { html: string; kind: 'html' }
  | { kind: 'empty' }
  | { kind: 'error' }
  | {
      kind: 'omitted'
      reason: Extract<ReviewDiffOmittedReason, 'hunks' | 'lines' | 'output'>
    }

type CodeTextMetrics = {
  bytes: number
  lines: number
  maxLineBytes: number
}

export function codeTextMetrics(text: string): CodeTextMetrics {
  const encoded = new TextEncoder().encode(text)
  if (encoded.length === 0) {
    return { bytes: 0, lines: 0, maxLineBytes: 0 }
  }

  let lineBytes = 0
  let lines = 0
  let maxLineBytes = 0
  for (const byte of encoded) {
    if (byte === 10) {
      lines += 1
      maxLineBytes = Math.max(maxLineBytes, lineBytes)
      lineBytes = 0
    } else {
      lineBytes += 1
    }
  }
  if (encoded.at(-1) !== 10) {
    lines += 1
    maxLineBytes = Math.max(maxLineBytes, lineBytes)
  }

  return { bytes: encoded.length, lines, maxLineBytes }
}
