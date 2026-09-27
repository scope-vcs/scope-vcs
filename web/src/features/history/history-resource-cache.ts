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

type HistoryScope = {
  scope: string
  audience: ProjectionPreviewAudience
  generation: string
  repoId: string
  viewKey: string
}

type HistoryFileIdentity = {
  path: string
  oldOid: string | null
  newOid: string | null
}

// Entry URLs stay stable across reprojection, so an entry is keyed by viewer
// scope and audience and refreshed through the repository change version.
export function historyEntryCacheKey(identity: {
  scope: string
  audience: ProjectionPreviewAudience
  entry: string
}) {
  const { scope, audience, entry } = identity
  return [scope, audience, entry].join('\0')
}

export function historyDiffCacheKey(identity: HistoryScope & HistoryFileIdentity & { commit: string }) {
  const { scope, repoId, generation, viewKey, audience, commit } = identity
  return [
    scope, repoId, generation, viewKey, audience, commit,
    identity.path,
    identity.oldOid ?? '',
    identity.newOid ?? '',
  ].join('\0')
}

// Blob ids pin the diff content, so a changed entry never reuses a stale diff.
export function historyEntryDiffCacheKey(identity: HistoryFileIdentity & {
  scope: string
  audience: ProjectionPreviewAudience
  entry: string
  visibilityChange: string | null
}) {
  return [
    historyEntryCacheKey(identity),
    identity.path,
    identity.oldOid ?? '',
    identity.newOid ?? '',
    identity.visibilityChange ?? '',
  ].join('\0')
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
