import { useMatches, useRouter } from '@tanstack/react-router'
import type { ComponentType, ReactNode } from 'react'

type PendingComponent = ComponentType<{ children?: ReactNode }>

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
