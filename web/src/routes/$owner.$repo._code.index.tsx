import { RepoContentError } from '@/components/repo-content-error'
import { RepoHomePage } from '@/features/repo-detail/repo-home-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { useRepoCodeResources } from '@/features/repo-detail/use-repo-code-resources'
import { createFileRoute } from '@tanstack/react-router'

const MAIN = { kind: 'main' } as const

export const Route = createFileRoute('/$owner/$repo/_code/')({
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoHomeRoute,
})

function RepoHomeRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const code = useRepoCodeResources({
    file: null,
    params,
    repo,
    version: MAIN,
  })
  return <RepoHomePage {...code} params={params} repo={repo} />
}
