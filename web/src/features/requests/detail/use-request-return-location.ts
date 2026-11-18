import { useLocation } from '@tanstack/react-router'
import { useState } from 'react'

export function useRequestReturnLocation(scope: string | null, requestsPath: string) {
  const location = useLocation()
  const [remembered, setRemembered] = useState<{
    scope: string | null
    location: typeof location | null
  }>({ scope: null, location: null })
  const inRequests = location.pathname === requestsPath || location.pathname.startsWith(`${requestsPath}/`)
  const selected = scope && inRequests && location.pathname !== requestsPath && location.pathname !== `${requestsPath}/`
    ? location
    : null

  if (remembered.scope !== scope || (inRequests && remembered.location !== selected)) {
    const next = { scope, location: selected }
    setRemembered(next)
    return next.location
  }
  return remembered.location
}
