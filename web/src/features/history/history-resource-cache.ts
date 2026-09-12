import type { ReviewFileDiff } from '@/api/types'
import type {
  HistoryEntryDetailResponse,
  ProjectionPreviewAudience,
} from '@/api/types.generated'
import { createBoundedCache } from '../../lib/bounded-cache'
import { createCachedResource } from '../../lib/cached-resource'

const MAX_ENTRY_ENTRIES = 48
const MAX_ENTRY_BYTES = 4 * 1024 * 1024
const MAX_DIFF_ENTRIES = 20
const MAX_DIFF_BYTES = 32 * 1024 * 1024

export const historyEntryResource = createCachedResource<HistoryEntryDetailResponse>({
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
  audience,
  entry,
  generation,
  repoId,
  viewKey,
}: {
  audience: ProjectionPreviewAudience
  entry: string
  generation: string
  repoId: string
  viewKey: string
}) {
  return [repoId, generation, viewKey, audience, entry].join('\0')
}

export function historyDiffCacheKey({
  audience,
  commit,
  generation,
  newOid,
  oldOid,
  path,
  repoId,
  viewKey,
}: {
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
  audience,
  entry,
  generation,
  newOid,
  oldOid,
  path,
  repoId,
  viewKey,
  visibilityChange = null,
}: {
  audience: ProjectionPreviewAudience
  entry: string
  generation: string
  newOid: string | null
  oldOid: string | null
  path: string
  repoId: string
  visibilityChange?: string | null
  viewKey: string
}) {
  return [historyDiffCacheKey({
    audience,
    commit: entry,
    generation,
    newOid,
    oldOid,
    path,
    repoId,
    viewKey,
  }), visibilityChange ?? ''].join('\0')
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
