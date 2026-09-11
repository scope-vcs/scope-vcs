import { parseRepoParams } from './repo-params'
import { parseFilePath } from './file-path-input'
import type {
  HistoryEntryDetailInput,
  HistoryEntryFileDiffInput,
  HistoryPageInput,
  ProjectionPreviewAudience,
} from './types'

export function parseHistoryPageInput(input: unknown): HistoryPageInput {
  return {
    ...parseRepoParams(input),
    audience: parseOptionalAudience(input),
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
    audience: parseOptionalAudience(input),
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

export function parseHistoryAudience(
  audience: unknown,
): ProjectionPreviewAudience {
  if (audience === 'private' || audience === 'public') {
    return audience
  }
  throw new Error(`Unsupported history audience: ${String(audience)}`)
}

function parseOptionalAudience(input: unknown): ProjectionPreviewAudience | null {
  const audience = (input as { audience?: unknown } | null)?.audience
  if (audience === undefined || audience === null || audience === '') {
    return null
  }
  return parseHistoryAudience(audience)
}

function parseOptionalBefore(input: unknown) {
  const data = input as Partial<HistoryPageInput> | null
  if (typeof data?.before !== 'string') return null
  return data.before.trim() || null
}

export function parseHistoryFeed(value: unknown): 'updates' | 'all' {
  if (value === undefined || value === null || value === '') return 'updates'
  if (value === 'updates' || value === 'all') return value
  throw new Error(`Unsupported history feed: ${String(value)}`)
}

export function parseVisibilityChange(value: unknown): string | null {
  if (value === undefined || value === null || value === '') return null
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('A visibility change id must be a non-empty string.')
  }
  return value.trim()
}
