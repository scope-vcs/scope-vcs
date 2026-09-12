import { createRouter } from '@tanstack/react-router'
import { MAIN_CONTENT_ID } from './components/main-content'
import { PendingSurface } from './components/pending-surface'
import { routeTree } from './routeTree.gen'

export function getRouter() {
  return createRouter({
    routeTree,
    defaultPendingComponent: PendingSurface,
    defaultPendingMinMs: 250,
    defaultPendingMs: 150,
    defaultPreload: 'intent',
    scrollRestoration: true,
    scrollToTopSelectors: [`#${MAIN_CONTENT_ID}`],
  })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
