import type {
  HistoryEntryDetail,
  ProjectionPreviewAudience,
  ReviewFileDiff,
} from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'
import { createCachedResource } from '../../lib/cached-resource'

const MAX_ENTRY_ENTRIES = 48
const MAX_ENTRY_BYTES = 4 * 1024 * 1024
const MAX_DIFF_ENTRIES = 20
const MAX_DIFF_BYTES = 32 * 1024 * 1024

export const historyEntryResource = createCachedResource<HistoryEntryDetail>({
  maxEntries: MAX_ENTRY_ENTRIES,
  maxWeight: MAX_ENTRY_BYTES,
  weightOf: approximateSerializedBytes,
})
export const historyDiffResource = createCachedResource<ReviewFileDiff>({
  maxEntries: MAX_DIFF_ENTRIES,
  maxWeight: MAX_DIFF_BYTES,
  weightOf: approximateSerializedBytes,
})

const diffScroll = createBoundedCache<string, number>({ maxEntries: MAX_DIFF_ENTRIES })

export function historyEntryCacheKey({
  scope,
  audience,
  entry,
  generation,
  repoId,
  viewKey,
}: {
  scope: string
  audience: ProjectionPreviewAudience
  entry: string
  generation: string
  repoId: string
  viewKey: string
}) {
  return [scope, repoId, generation, viewKey, audience, entry].join('\0')
}

export function historyDiffCacheKey({
  scope,
  audience,
  commit,
  generation,
  newOid,
  oldOid,
  path,
  repoId,
  viewKey,
}: {
  scope: string
  audience: ProjectionPreviewAudience
  commit: string
  generation: string
  newOid: string | null
  oldOid: string | null
  path: string
  repoId: string
  viewKey: string
}) {
  return [
    scope,
    repoId,
    generation,
    viewKey,
    audience,
    commit,
    path,
    oldOid ?? '',
    newOid ?? '',
  ].join('\0')
}

export function historyEntryDiffCacheKey({
  scope,
  audience,
  entry,
  generation,
  newOid,
  oldOid,
  path,
  repoId,
  viewKey,
  visibilityChange = null,
  commitOid = null,
}: {
  scope: string
  audience: ProjectionPreviewAudience
  entry: string
  generation: string
  newOid: string | null
  oldOid: string | null
  path: string
  repoId: string
  visibilityChange?: string | null
  commitOid?: string | null
  viewKey: string
}) {
  return [historyDiffCacheKey({
    scope,
    audience,
    commit: entry,
    generation,
    newOid,
    oldOid,
    path,
    repoId,
    viewKey,
  }), visibilityChange ?? '', commitOid ?? ''].join('\0')
}

export function readHistoryDiffScroll(key: string | null) {
  if (!key || !historyDiffResource.peek(key)) return 0
  return diffScroll.peek(key) ?? 0
}

export function writeHistoryDiffScroll(key: string | null, scrollTop: number) {
  if (!key) return
  if (historyDiffResource.peek(key)) diffScroll.set(key, scrollTop)
}

export function resetHistoryResourceCache() {
  historyEntryResource.clear()
  historyDiffResource.clear()
  diffScroll.clear()
}

export function historyResourceCacheStats() {
  const diffs = historyDiffResource.stats()
  const entries = historyEntryResource.stats()
  return {
    diffBytes: diffs.totalWeight,
    diffs: diffs.entries,
    entryBytes: entries.totalWeight,
    entries: entries.entries,
  }
}

function approximateSerializedBytes(value: unknown) {
  return JSON.stringify(value).length * 2
}
