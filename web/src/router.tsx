import { createRouter } from '@tanstack/react-router'
import { MAIN_CONTENT_ID } from './components/main-content'
import { PendingSurface } from './components/pending-surface'
import { routeTree } from './routeTree.gen'

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
  })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
