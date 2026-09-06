import type { RepoContent } from '@/api/types'
import { createCachedResource } from '../../lib/cached-resource'

export const repoContentResource = createCachedResource<RepoContent>({
  maxEntries: 8,
  maxWeight: 8 * 1024 * 1024,
  weightOf: approximateContentBytes,
})

export function readRepoContentCache(key: string) {
  return repoContentResource.read(key) ?? null
}

export function writeRepoContentCache(key: string, content: RepoContent) {
  repoContentResource.write(key, content)
}

export function repoContentCacheKey({
  audience,
  changeVersion,
  repoId,
}: {
  audience: 'private' | 'public'
  changeVersion: number
  repoId: string
}) {
  return [repoId, changeVersion, audience].join('\0')
}

export function resetRepoContentCache() {
  repoContentResource.clear()
}

export function repoContentCacheStats() {
  return repoContentResource.stats()
}

function approximateContentBytes(content: RepoContent) {
  return content.clone_remote_url.length * 2 + content.files.reduce(
    (bytes, file) => bytes + file.path.length * 2 + file.oid.length * 2 + 32,
    0,
  )
}
