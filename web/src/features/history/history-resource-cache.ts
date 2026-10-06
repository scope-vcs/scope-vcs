import type { ReviewFileDiff } from '@/api/types'
import type {
  HistoryEntryDetailResponse,
  ViewId,
} from '@/api/types.generated'
import { createBoundedCache } from '../../lib/bounded-cache'
import { createCachedResource } from '../../lib/cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import type { LoadedHistory } from './history-pagination'

const MAX_ENTRY_ENTRIES = 48
const MAX_ENTRY_BYTES = 4 * 1024 * 1024
const MAX_DIFF_ENTRIES = 20
const MAX_DIFF_BYTES = 32 * 1024 * 1024

export const historyFeedResource = createCachedResource<LoadedHistory>({
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

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
onViewerChange(() => diffScroll.clear())

type HistoryScope = {
  scope: string
  view: ViewId
  generation: string
  repoId: string
  revisionKey: string
}

type HistoryFileIdentity = {
  path: string
  oldOid: string | null
  newOid: string | null
}

export function historyEntryCacheKey(identity: {
  scope: string
  view: ViewId
  entry: string
}) {
  const { scope, view, entry } = identity
  return [scope, view, entry].join('\0')
}

export function historyDiffCacheKey(identity: HistoryScope & HistoryFileIdentity & { commit: string }) {
  const { scope, repoId, generation, revisionKey, view, commit } = identity
  return [
    scope, repoId, generation, revisionKey, view, commit,
    identity.path,
    identity.oldOid ?? '',
    identity.newOid ?? '',
  ].join('\0')
}

export function historyEntryDiffCacheKey(identity: HistoryFileIdentity & {
  scope: string
  view: ViewId
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

function approximateSerializedBytes(value: unknown) {
  return JSON.stringify(value).length * 2
}
