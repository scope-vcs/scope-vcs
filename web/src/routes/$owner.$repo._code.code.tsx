import { RepoContentError } from '@/components/repo-content-error'
import { RepoCodePage } from '@/features/repo-detail/repo-code-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import {
  parseRepositoryVersionSearch,
  repositoryVersion,
  type RepositoryVersionSearch,
} from '@/features/repo-detail/repository-version'
import { useRepoCodeResources } from '@/features/repo-detail/use-repo-code-resources'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'

type RepoCodeSearch = RepositoryVersionSearch & { file?: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { ...parseRepositoryVersionSearch(search), file: parseRouteFileSearch(search.file) }
}

export const Route = createFileRoute('/$owner/$repo/_code/code')({
  validateSearch: parseRepoCodeSearch,
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoCodeRoute,
})

function RepoCodeRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const { request, update } = search
  const version = useMemo(() => repositoryVersion({ request, update }), [request, update])
  const code = useRepoCodeResources({
    file: search.file ?? null,
    params,
    repo,
    version,
  })
  const selectedPath = code.selectedPath
  const selectFile = useCallback((path: string) => {
    const nextPath = displayRouteFilePath(path)
    if (nextPath === selectedPath) return
    void navigate({
      resetScroll: false,
      search: (current) => ({ ...current, file: nextPath }),
    })
  }, [navigate, selectedPath])

  return (
    <RepoCodePage
      {...code}
      onSelectFilePath={selectFile}
      params={params}
      repo={repo}
      version={version}
    />
  )
}
