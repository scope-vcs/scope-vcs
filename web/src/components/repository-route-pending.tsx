import { RepoShell } from '@/components/repo-shell'
import { useMatches, useParams, useRouter } from '@tanstack/react-router'
import type { ComponentType, ReactNode } from 'react'

type PendingComponent = ComponentType<{ children?: ReactNode }>

export function RepositoryRoutePending() {
  const params = useParams({ from: '/$owner/$repo' })
  return (
    <RepoShell params={params} repo={null}>
      <SectionRoutesPending />
    </RepoShell>
  )
}

// While the repository loads, its child routes are already matched. Each one
// that owns a pending state renders it, nested the way the routes nest, so a
// section looks the same whether you enter the repository or switch to it.
function SectionRoutesPending() {
  const router = useRouter()
  const routeIds = useMatches({ select: (matches) => matches.map((match) => match.routeId) })
  const sectionRouteIds = routeIds.slice(routeIds.indexOf('/$owner/$repo') + 1)
  const pending = sectionRouteIds.flatMap((routeId) => {
    const component = router.routesById[routeId]?.options.pendingComponent
    return component ? [component as PendingComponent] : []
  })
  return pending.reduceRight<ReactNode>((child, Pending) => <Pending>{child}</Pending>, null)
}
