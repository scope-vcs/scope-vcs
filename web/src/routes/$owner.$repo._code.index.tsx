import { parseRepoFileInput } from '@/api/request-inputs'
import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoContent, RepoFileContent, RepoSummary } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import {
  repoContentResource,
  repoContentCacheKey,
} from '@/features/repo-detail/repo-content-cache'
import {
  repoFileResource,
  repoFileCacheKey,
} from '@/features/repo-detail/repo-file-cache'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import {
  repositoryLandingPath,
  loadRepoFileWhenReady,
  type RepoFileLoadResult,
} from '@/features/repo-detail/repo-code-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  createFileRoute,
  useNavigate,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { useCallback } from 'react'
import { useAuth } from '@clerk/tanstack-react-start'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data, getRequest().signal))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator(parseRepoFileInput)
  .handler(async ({ data }): Promise<RepoFileLoadResult> => {
    try {
      return { file: await loadRepoFileForRequest(data, getRequest().signal), status: 'ready' }
    } catch (error) {
      if (error instanceof HttpError && error.status === 404) {
        return { status: 'missing' }
      }
      if (
        error instanceof HttpError &&
        error.status === 503 &&
        error.message === PROJECTION_REBUILDING_MESSAGE
      ) {
        return { status: 'rebuilding' }
      }
      throw error
    }
  })

export const Route = createFileRoute('/$owner/$repo/_code/')({
  validateSearch: parseRepoCodeSearch,
  errorComponent: RepoContentError,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const { userId, isLoaded } = useAuth()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const owner = params.owner
  const repoName = params.repo
  const { contentIdentity } = repoCodeCacheKeys(repo, scope, null)
  const loadContent = useCallback((signal: AbortSignal): Promise<RepoContent> => loadRepoContent({
    data: { owner, repo: repoName }, signal,
  }), [owner, repoName])
  const contentResource = useCachedResource({
    fallbackError: 'Repository files are unavailable.',
    identity: contentIdentity,
    load: loadContent,
    resource: repoContentResource,
  })
  const content = contentResource.value
  // A new version can remove file visibility. Revalidate the landing path from
  // the current tree instead of retaining the previous version's README.
  const selectedPath = search.file ?? (content ? repositoryLandingPath(content.files) : null)
  const { fileIdentity: selectedFileIdentity } = repoCodeCacheKeys(repo, scope, selectedPath)
  const loadSelectedFile = useCallback((signal: AbortSignal) => {
    if (!selectedPath) throw new Error('No file selected.')
    return loadAddressedFile({ owner, path: selectedPath, repo: repoName }, signal)
  }, [owner, repoName, selectedPath])
  const selectedFileResource = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity: selectedFileIdentity,
    load: loadSelectedFile,
    resource: repoFileResource,
  })
  const selectFile = useCallback((path: string) => {
    const nextPath = displayRouteFilePath(path)
    if (nextPath === selectedPath) return
    void navigate({
      resetScroll: false,
      search: {
        file: nextPath,
      },
    })
  }, [navigate, selectedPath])

  return (
    <RepoDetailPage
      content={content}
      contentError={contentResource.error}
      contentLoading={!isLoaded || contentResource.status === 'loading'}
      contentRetry={contentResource.retry}
      onSelectFilePath={selectFile}
      params={params}
      repo={repo}
      selectedFile={selectedFileResource.value}
      selectedFileError={selectedFileResource.error}
      selectedFileIdentity={selectedFileIdentity}
      selectedFileLoading={!isLoaded || selectedFileResource.status === 'loading'}
      selectedFileRetry={selectedFileResource.retry}
      selectedPath={selectedPath}
    />
  )
}

type RepoCodeSearch = { file?: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { file: parseRouteFileSearch(search.file) }
}

async function loadAddressedFile(
  data: ReturnType<typeof parseRepoFileInput>,
  signal: AbortSignal,
): Promise<RepoFileContent> {
  const file = await loadRepoFileWhenReady({
    load: () => loadRepoFile({ data, signal }),
    signal,
  })
  if (!file) throw new Error('This file is no longer available in the current scoped view.')
  return file
}

function repoCodeCacheKeys(repo: RepoSummary, scope: string | null, path: string | null) {
  if (!scope) return { contentIdentity: null, fileIdentity: null }
  const identity = {
    scope,
    audience: repo.access.can_read_private_files ? 'private' as const : 'public' as const,
    changeVersion: repo.change_version,
    repoId: repo.id,
  }
  return {
    contentIdentity: repoContentCacheKey(identity),
    fileIdentity: path ? repoFileCacheKey({ ...identity, path }) : null,
  }
}
