import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RepoShell } from '@/components/repo-shell'
import { useParams } from '@tanstack/react-router'

export function RepositoryRoutePending() {
  const params = useParams({ from: '/$owner/$repo' })
  return (
    <RepoShell params={params} repo={null}>
      <ChildRoutesPending below="/$owner/$repo" />
    </RepoShell>
  )
}
