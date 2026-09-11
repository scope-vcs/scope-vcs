import { displayRouteFilePath } from '../../lib/route-file'
import { createCachedResource } from '../../lib/cached-resource'
import type { RepoFileContentResponse } from '@/api/types.generated'

const MAX_CACHE_ENTRIES = 32
const MAX_CACHE_BYTES = 24 * 1024 * 1024

export const repoFileResource = createCachedResource<RepoFileContentResponse>({
  maxEntries: MAX_CACHE_ENTRIES,
  maxWeight: MAX_CACHE_BYTES,
  weightOf: approximateFileBytes,
})

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
  return [repoId, changeVersion, audience, displayRouteFilePath(path)].join('\0')
}

function approximateFileBytes(file: RepoFileContentResponse) {
  return file.content.kind === 'text'
    ? file.content.text.length * 2
    : file.content.size_bytes
}
