import { parseRepoParams } from './repo-params'
import { parseFilePath } from './file-path-input'
import type {
  HistoryEntryDetailInput,
  HistoryEntryFileDiffInput,
  HistoryPageInput,
} from './types'
import type { HistoryFeed, ViewId } from './types.generated'

export function parseHistoryPageInput(input: unknown): HistoryPageInput {
  return {
    ...parseRepoParams(input),
    view: parseOptionalView(input),
    before: parseOptionalBefore(input),
    feed: parseHistoryFeed((input as { feed?: unknown } | null)?.feed),
  }
}

export function parseHistoryEntryDetailInput(input: unknown): HistoryEntryDetailInput {
  const data = input as Partial<HistoryEntryDetailInput> | null
  const entry = typeof data?.entry === 'string' ? data.entry.trim() : ''
  if (!entry) {
    throw new Error('A history entry id is required.')
  }

  return {
    ...parseRepoParams(input),
    view: parseOptionalView(input),
    entry,
  }
}

export function parseHistoryEntryFileDiffInput(input: unknown): HistoryEntryFileDiffInput {
  const data = input as Partial<HistoryEntryFileDiffInput> | null
  return {
    ...parseHistoryEntryDetailInput(input),
    path: parseFilePath(data?.path),
    visibility_change: parseVisibilityChange(data?.visibility_change),
  }
}

export function parseHistoryView(
  view: unknown,
): ViewId {
  if (typeof view === 'string' && /^[a-z][a-z0-9_-]{0,31}$/.test(view)) {
    return view
  }
  throw new Error(`Unsupported history view: ${String(view)}`)
}

function parseOptionalView(input: unknown): ViewId | null {
  const view = (input as { view?: unknown } | null)?.view
  if (view === undefined || view === null || view === '') {
    return null
  }
  return parseHistoryView(view)
}

function parseOptionalBefore(input: unknown) {
  const data = input as Partial<HistoryPageInput> | null
  if (typeof data?.before !== 'string') return null
  return data.before.trim() || null
}

export function parseHistoryFeed(value: unknown): HistoryFeed {
  if (value === undefined || value === null || value === '') return 'updates'
  if (value === 'updates' || value === 'all' || value === 'visibility') return value
  throw new Error(`Unsupported history feed: ${String(value)}`)
}

export function parseVisibilityChange(value: unknown): string | null {
  if (value === undefined || value === null || value === '') return null
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('A visibility change id must be a non-empty string.')
  }
  return value.trim()
}
