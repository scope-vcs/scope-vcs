import type { ViewId } from '@/api/types.generated'
import { displayRouteFilePath } from '../../lib/route-file'
import { createRepoResource, repoResourceKey } from './repo-resource'
import { repositoryVersionKey, type RepositoryVersion } from './repository-version'
import type { RepoFile } from './repo-file-highlight'

const MAX_CACHE_ENTRIES = 32
const MAX_CACHE_BYTES = 24 * 1024 * 1024

export const repoFileResource = createRepoResource<RepoFile>({
  invalidatedBy: ['content', 'requestHead'],
  affectedBy: (change, key) => change.topic !== 'requestHead'
    || change.requestId === undefined
    || key[3] === repositoryVersionKey({ kind: 'request', id: change.requestId }),
  maxEntries: MAX_CACHE_ENTRIES,
  maxWeight: MAX_CACHE_BYTES,
  weightOf: approximateFileBytes,
})

export function repoFileCacheKey({
  scope,
  view,
  contentVersion,
  path,
  repoId,
  version,
}: {
  scope: string
  view: ViewId
  contentVersion: number
  path: string
  repoId: string
  version: RepositoryVersion
}) {
  return repoResourceKey(scope, repoId, contentVersion, view, repositoryVersionKey(version), displayRouteFilePath(path))
}

function approximateFileBytes(file: RepoFile) {
  const contentBytes = file.content.kind === 'text' ? file.content.text.length * 2 : 0
  return contentBytes + (file.highlightedHtml?.length ?? 0) * 2
}
