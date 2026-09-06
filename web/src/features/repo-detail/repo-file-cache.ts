import type { RepoFileContent } from '@/api/types'
import { createCachedResource } from '../../lib/cached-resource'

const MAX_CACHE_ENTRIES = 32
const MAX_CACHE_BYTES = 24 * 1024 * 1024

export const repoFileResource = createCachedResource<RepoFileContent>({
  maxEntries: MAX_CACHE_ENTRIES,
  maxWeight: MAX_CACHE_BYTES,
  weightOf: approximateFileBytes,
})

export function readRepoFileCache(key: string) {
  return repoFileResource.read(key) ?? null
}

export function writeRepoFileCache(key: string, file: RepoFileContent) {
  repoFileResource.write(key, file)
}

export function repoFileCacheKey({
  audience,
  changeVersion,
  path,
  repoId,
}: {
  audience: 'private' | 'public'
  changeVersion: number
  path: string
  repoId: string
}) {
  return [repoId, changeVersion, audience, path.replace(/^\/+/, '')].join('\0')
}

export function resetRepoFileCache() {
  repoFileResource.clear()
}

export function repoFileCacheStats() {
  const stats = repoFileResource.stats()
  return { entries: stats.entries, totalBytes: stats.totalWeight }
}

function approximateFileBytes(file: RepoFileContent) {
  return file.content.kind === 'text'
    ? file.content.text.length * 2
    : file.content.size_bytes
}
