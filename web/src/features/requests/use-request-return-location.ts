import { useLocation } from '@tanstack/react-router'
import { useState } from 'react'

/** The repository shell stays mounted while switching between Requests and Runs. */
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

  // Visiting the list explicitly clears the selection. Changing viewer, repository,
  // or access scope discards the old destination before it can become a link.
  if (remembered.scope !== scope || (inRequests && remembered.location !== selected)) {
    const next = { scope, location: selected }
    setRemembered(next)
    return next.location
  }
  return remembered.location
}
