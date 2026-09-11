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

type HistoryScope = {
  scope: string
  audience: ProjectionPreviewAudience
  generation: string
  repoId: string
  viewKey: string
}

type HistoryFileIdentity = HistoryScope & {
  path: string
  oldOid: string | null
  newOid: string | null
}

export function historyEntryCacheKey(identity: HistoryScope & { entry: string }) {
  const { scope, repoId, generation, viewKey, audience, entry } = identity
  return [scope, repoId, generation, viewKey, audience, entry].join('\0')
}

export function historyDiffCacheKey(identity: HistoryFileIdentity & { commit: string }) {
  return [
    historyEntryCacheKey({ ...identity, entry: identity.commit }),
    identity.path,
    identity.oldOid ?? '',
    identity.newOid ?? '',
  ].join('\0')
}

export function historyEntryDiffCacheKey(identity: HistoryFileIdentity & {
  entry: string
  visibilityChange?: string | null
}) {
  return [historyDiffCacheKey({ ...identity, commit: identity.entry }), identity.visibilityChange ?? ''].join('\0')
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

function approximateSerializedBytes(value: unknown) {
  return JSON.stringify(value).length * 2
}
