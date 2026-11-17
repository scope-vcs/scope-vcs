import { createRouter } from '@tanstack/react-router'
import { createIsomorphicFn } from '@tanstack/react-start'
import { getRequestHeader } from '@tanstack/react-start/server'
import { MAIN_CONTENT_ID } from './components/main-content'
import { PendingSurface } from './components/pending-surface'
import { routeTree } from './routeTree.gen'
import { CSP_NONCE_HEADER } from './server/security-headers'

const responseScriptNonce = createIsomorphicFn()
  .server(() => getRequestHeader(CSP_NONCE_HEADER))
  .client(() => undefined)

export function getRouter() {
  let renderedEntry: string | undefined
  return createRouter({
    routeTree,
    defaultPendingComponent: PendingSurface,
    defaultPendingMinMs: 250,
    defaultPendingMs: 150,
    defaultPreload: 'intent',
    scrollRestoration: ({ location }) => {
      const entry = location.state.__TSR_key ?? location.href
      const entering = entry !== renderedEntry
      renderedEntry = entry
      return entering
    },
    scrollToTopSelectors: [`#${MAIN_CONTENT_ID}`],
    ssr: { nonce: responseScriptNonce() },
  })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
