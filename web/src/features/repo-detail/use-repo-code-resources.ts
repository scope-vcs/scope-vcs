import type { RepoContent } from '@/api/types'
import type {
  RepoFileContentResponse,
  RepoSummaryResponse,
  ViewId,
} from '@/api/types.generated'
import { displayRouteFilePath } from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadRepoContent, loadRepoFile } from '@/routes/-repo-code-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useMemo } from 'react'
import { repoContentCacheKey, repoContentResource } from './repo-content-cache'
import {
  loadRepoFileWhenReady,
  repoCodeResourceLoader,
  repositoryLandingPath,
  settleRepoCodeResource,
} from './repo-code-route-data'
import { repoFileCacheKey, repoFileResource } from './repo-file-cache'
import { repoResourceScope } from './repo-resource-scope'
import {
  repositoryVersionSearch,
  type RepositoryVersion,
} from './repository-version'
import { useViewingAs } from './use-viewing-as'

type RepoCodeParams = { owner: string; repo: string }

export type RepoCodeLoaderData = {
  content: ReturnType<typeof settleRepoCodeResource<RepoContent>> | null
  contentIdentity: string | null
  file: ReturnType<typeof settleRepoCodeResource<RepoFileContentResponse>> | null
  fileIdentity: string | null
}

export const EMPTY_REPO_CODE_LOADER_DATA: RepoCodeLoaderData = {
  content: null,
  contentIdentity: null,
  file: null,
  fileIdentity: null,
}

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

export function prepareRepoCodeLoaderData({
  file,
  keys,
  params,
  signal,
  version,
  view,
}: {
  file: string | null
  keys: ReturnType<typeof repoCodeCacheKeys>
  params: RepoCodeParams
  signal: AbortSignal
  version: RepositoryVersion
  view: ViewId
}): RepoCodeLoaderData {
  const content = loadRepoContent({
    data: { ...params, ...repositoryVersionSearch(version), view },
    signal,
  })
  return {
    content: settleRepoCodeResource(content),
    contentIdentity: keys.contentIdentity,
    file: file && keys.fileIdentity
      ? settleRepoCodeResource(loadAddressedFile(params, file, version, view, signal))
      : null,
    fileIdentity: keys.fileIdentity,
  }
}

export function useRepoCodeResources({
  file,
  page,
  params,
  repo,
  version,
}: {
  file: string | null
  page: RepoCodeLoaderData
  params: RepoCodeParams
  repo: RepoSummaryResponse
  version: RepositoryVersion
}) {
  const { userId, isLoaded } = useAuth()
  const { view } = useViewingAs()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const refreshVersion = version.kind === 'request' ? String(repo.change_version) : ''
  const { contentIdentity } = repoCodeCacheKeys(repo, scope, view, null, version)
  const loadContent = useMemo(() => repoCodeResourceLoader(
    page.contentIdentity === contentIdentity ? page.content : null,
    (signal: AbortSignal): Promise<RepoContent> => loadRepoContent({
      data: { owner, repo: repoName, view, ...repositoryVersionSearch(version) }, signal,
    }),
  ), [contentIdentity, owner, page.content, page.contentIdentity, repoName, version, view])
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
  const loadSelectedFile = useMemo(() => repoCodeResourceLoader(
    page.fileIdentity === fileIdentity ? page.file : null,
    (signal: AbortSignal) => {
      if (!selectedPath) throw new Error('No file selected.')
      return loadAddressedFile({ owner, repo: repoName }, selectedPath, version, view, signal)
    },
  ), [fileIdentity, owner, page.file, page.fileIdentity, repoName, selectedPath, version, view])
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
): Promise<RepoFileContentResponse> {
  const data = { ...params, ...repositoryVersionSearch(version), path, view }
  const file = await loadRepoFileWhenReady({
    load: () => loadRepoFile({ data, signal }),
    signal,
  })
  if (!file) throw new Error('This file is no longer available in the current scoped view.')
  return file
}
