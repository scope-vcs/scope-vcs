import type { RepoContent } from '@/api/types'
import { createCachedResource } from '../../lib/cached-resource'

export const repoContentResource = createCachedResource<RepoContent>({
  maxEntries: 8,
  maxWeight: 8 * 1024 * 1024,
  weightOf: approximateContentBytes,
})

export type RepoContentIdentity = {
  scope: string
  audience: 'private' | 'public'
  changeVersion: number
  repoId: string
}

export function repoContentCacheKey(identity: RepoContentIdentity) {
  const { scope, repoId, changeVersion, audience } = identity
  return [scope, repoId, changeVersion, audience].join('\0')
}

function approximateContentBytes(content: RepoContent) {
  return content.clone_remote_url.length * 2 + content.files.reduce(
    (bytes, file) => bytes + file.path.length * 2 + file.oid.length * 2 + 32,
    0,
  )
}
