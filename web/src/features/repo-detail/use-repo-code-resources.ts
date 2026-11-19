import type { RepoContent } from '@/api/types'
import type {
  RepoSummaryResponse,
  ViewId,
} from '@/api/types.generated'
import { displayRouteFilePath } from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadRepoContent, loadRepoFile } from '@/routes/-repo-code-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useMemo } from 'react'
import { repoContentCacheKey, repoContentResource } from './repo-content-cache'
import { loadRepoFileWhenReady, repositoryLandingPath } from './repo-code-route-data'
import { repoFileCacheKey, repoFileResource } from './repo-file-cache'
import type { RepoFile } from './repo-file-highlight'
import { repoResourceScope } from './repo-resource-scope'
import {
  repositoryVersionSearch,
  type RepositoryVersion,
} from './repository-version'
import { useViewingAs } from './use-viewing-as'

type RepoCodeParams = { owner: string; repo: string }

export function repoCodeCacheKeys(
  repo: RepoSummaryResponse,
  accessScope: string | null,
  view: ViewId,
  path: string | null,
  version: RepositoryVersion,
) {
  if (!accessScope) return { contentIdentity: null, fileIdentity: null }
  const scope = {
    scope: accessScope,
    view,
    contentVersion: repo.content_version,
    repoId: repo.id,
    version,
  }
  return {
    contentIdentity: repoContentCacheKey(scope),
    fileIdentity: path ? repoFileCacheKey({ ...scope, path }) : null,
  }
}

export function repoCodeRefreshVersion(repo: RepoSummaryResponse, version: RepositoryVersion) {
  return version.kind === 'request' ? String(repo.change_version) : ''
}

export function useRepoCodeResources({
  file,
  params,
  repo,
  version,
}: {
  file: string | null
  params: RepoCodeParams
  repo: RepoSummaryResponse
  version: RepositoryVersion
}) {
  const { userId, isLoaded } = useAuth()
  const { view } = useViewingAs()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const refreshVersion = repoCodeRefreshVersion(repo, version)
  const { contentIdentity } = repoCodeCacheKeys(repo, scope, view, null, version)
  const loadContent = useCallback((signal: AbortSignal): Promise<RepoContent> => loadRepoContent({
    data: { owner, repo: repoName, view, ...repositoryVersionSearch(version) }, signal,
  }), [owner, repoName, version, view])
  const content = useCachedResource({
    fallbackError: 'Repository files are unavailable.',
    identity: contentIdentity,
    load: loadContent,
    resource: repoContentResource,
    version: refreshVersion,
  })
  const files = content.value?.files
  const selectedPath = useMemo(() => {
    if (!files) return file
    if (file && files.some((candidate) => displayRouteFilePath(candidate.path) === file)) return file
    return repositoryLandingPath(files)
  }, [file, files])
  const { fileIdentity } = repoCodeCacheKeys(repo, scope, view, selectedPath, version)
  const loadSelectedFile = useCallback((signal: AbortSignal) => {
    if (!selectedPath) throw new Error('No file selected.')
    return loadAddressedFile({ owner, repo: repoName }, selectedPath, version, view, signal)
  }, [owner, repoName, selectedPath, version, view])
  const selectedFile = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity: fileIdentity,
    load: loadSelectedFile,
    resource: repoFileResource,
    version: refreshVersion,
  })
  return { content, file: selectedFile, selectedPath }
}

async function loadAddressedFile(
  params: RepoCodeParams,
  path: string,
  version: RepositoryVersion,
  view: ViewId,
  signal: AbortSignal,
): Promise<RepoFile> {
  const data = { ...params, ...repositoryVersionSearch(version), path, view }
  const file = await loadRepoFileWhenReady({
    load: () => loadRepoFile({ data, signal }),
    signal,
  })
  if (!file) throw new Error('This file is no longer available in the current scoped view.')
  return file
}
