import { useMatches, useRouter } from '@tanstack/react-router'
import type { ComponentType, ReactNode } from 'react'

type PendingComponent = ComponentType<{ children?: ReactNode }>

/**
 * While a route pends, the routes below it are already matched. This renders
 * each one's pending state, nested the way the routes nest, so a skeleton shows
 * the shape of the page it leads to however the visitor arrived.
 */
export function ChildRoutesPending({ below, fallback = null }: { below: string; fallback?: ReactNode }) {
  const router = useRouter()
  const routeIds = useMatches({ select: (matches) => matches.map((match) => match.routeId) })
  const start = routeIds.indexOf(below as (typeof routeIds)[number])
  const pending = start < 0 ? [] : routeIds.slice(start + 1).flatMap((routeId) => {
    const component = router.routesById[routeId]?.options.pendingComponent
    return component ? [component as PendingComponent] : []
  })
  if (!pending.length) return fallback
  return pending.reduceRight<ReactNode>((child, Pending) => <Pending>{child}</Pending>, null)
}
