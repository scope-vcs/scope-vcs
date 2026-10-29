import type { ReviewDiffOmittedReason, ReviewDiffRenderBudget } from '../../api/types'

export const REVIEW_FILE_DIFF_RENDERER_LIMITS = Object.freeze({
  maxConcurrentRenders: 2,
  maxMixedTextBytes: 64 * 1024,
  startupDeadlineMs: 10_000,
})

export type ReviewFileDiffRenderBudget = Readonly<{
  deadlineMs: number
  maxHunks: number
  maxInputBytes: number
  maxInputLineBytes: number
  maxInputLines: number
  maxOutputBytes: number
  maxRenderedLines: number
}>

export const REVIEW_FILE_DIFF_RENDER_BUDGETS: Readonly<Record<
  ReviewDiffRenderBudget,
  ReviewFileDiffRenderBudget
>> = Object.freeze({
  full: Object.freeze({
    deadlineMs: 10_000,
    maxHunks: 2_000,
    maxInputBytes: 2 * 1024 * 1024,
    maxInputLineBytes: 1024 * 1024,
    maxInputLines: 200_000,
    maxOutputBytes: 8 * 1024 * 1024,
    maxRenderedLines: 20_000,
  }),
  standard: Object.freeze({
    deadlineMs: 1_500,
    maxHunks: 64,
    maxInputBytes: 256 * 1024,
    maxInputLineBytes: 16 * 1024,
    maxInputLines: 4_000,
    maxOutputBytes: 256 * 1024,
    maxRenderedLines: 800,
  }),
})

export type ReviewFileDiffWorkerInput = {
  budget: Pick<
    ReviewFileDiffRenderBudget,
    'maxHunks' | 'maxOutputBytes' | 'maxRenderedLines'
  >
  newText: string
  oldText: string
  path: string
}

export type ReviewFileDiffWorkerResult =
  | { html: string; kind: 'html' }
  | { kind: 'empty' }
  | { kind: 'error' }
  | {
      kind: 'omitted'
      reason: Extract<ReviewDiffOmittedReason, 'hunks' | 'lines' | 'output'>
    }

type ReviewFileTextMetrics = {
  bytes: number
  lines: number
  maxLineBytes: number
}

export function reviewFileTextMetrics(text: string): ReviewFileTextMetrics {
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
