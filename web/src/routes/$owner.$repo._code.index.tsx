import { parseRepoFileInput } from '@/api/request-inputs'
import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoContent, RepoFileContent, RepoLiveState, RepoSummary } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import {
  peekRepoContentCache,
  readRepoContentCache,
  repoContentCacheKey,
  writeRepoContentCache,
} from '@/features/repo-detail/repo-content-cache'
import {
  peekRepoFileCache,
  readRepoFileCache,
  repoFileCacheKey,
  writeRepoFileCache,
} from '@/features/repo-detail/repo-file-cache'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import {
  repositoryLandingPath,
  loadRepoFileWhenReady,
  settleRepoCodeResource,
  repoCodeResourceLoader,
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
import { useCallback, useMemo } from 'react'

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
  loaderDeps: ({ search }) => ({ file: search.file ?? null }),
  loader: async ({ abortController, deps, params, parentMatchPromise }) => {
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { contentIdentity, fileIdentity } = repoCodeCacheKeys(live.repo, deps.file)
    const cachedContent = typeof window === 'undefined' ? null : readRepoContentCache(contentIdentity)
    const cachedFile = typeof window === 'undefined' || !fileIdentity ? null : readRepoFileCache(fileIdentity)
    const signal = abortController.signal
    return {
      content: settleRepoCodeResource(cachedContent
        ? Promise.resolve(cachedContent)
        : loadRepoContent({ data: params, signal })),
      file: deps.file ? settleRepoCodeResource(cachedFile
        ? Promise.resolve(cachedFile)
        : loadAddressedFile({ ...params, path: deps.file }, signal)) : null,
      contentIdentity,
      fileIdentity,
    }
  },
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const page = Route.useLoaderData()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const owner = params.owner
  const repoName = params.repo
  const { contentIdentity } = repoCodeCacheKeys(repo, null)
  const loadContent = useMemo(() => repoCodeResourceLoader(
    page.contentIdentity === contentIdentity ? page.content : null,
    (signal: AbortSignal): Promise<RepoContent> => loadRepoContent({
      data: { owner, repo: repoName }, signal,
    }),
  ), [contentIdentity, owner, page.content, page.contentIdentity, repoName])
  const contentResource = useCachedResource({
    fallbackError: 'Repository files are unavailable.',
    identity: contentIdentity,
    load: loadContent,
    peek: peekRepoContentCache,
    read: readRepoContentCache,
    write: writeRepoContentCache,
  })
  const content = contentResource.value
  const selectedPath = search.file ?? (content ? repositoryLandingPath(content.files) : null)
  const { fileIdentity: selectedFileIdentity } = repoCodeCacheKeys(repo, selectedPath)
  const loadSelectedFile = useMemo(() => repoCodeResourceLoader(
    page.fileIdentity === selectedFileIdentity ? page.file : null,
    (signal: AbortSignal) => {
      if (!selectedPath) throw new Error('No file selected.')
      return loadAddressedFile({ owner, path: selectedPath, repo: repoName }, signal)
    },
  ), [owner, page.file, page.fileIdentity, repoName, selectedFileIdentity, selectedPath])
  const selectedFileResource = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity: selectedFileIdentity,
    load: loadSelectedFile,
    peek: peekRepoFileCache,
    read: readRepoFileCache,
    write: writeRepoFileCache,
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
      contentLoading={contentResource.status === 'loading'}
      contentRetry={contentResource.retry}
      onSelectFilePath={selectFile}
      params={params}
      repo={repo}
      selectedFile={selectedFileResource.value}
      selectedFileError={selectedFileResource.error}
      selectedFileIdentity={selectedFileIdentity}
      selectedFileLoading={selectedFileResource.status === 'loading'}
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

function repoCodeCacheKeys(repo: RepoSummary, path: string | null) {
  const scope = {
    audience: repo.access.can_read_private_files ? 'private' as const : 'public' as const,
    changeVersion: repo.change_version,
    repoId: repo.id,
  }
  return {
    contentIdentity: repoContentCacheKey(scope),
    fileIdentity: path ? repoFileCacheKey({ ...scope, path }) : null,
  }
}
