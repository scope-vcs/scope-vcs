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
    // The router restores scroll after every render, including reloads of the
    // entry being read, such as live refreshes. Those would move the reader
    // back to where the reload began, so only entering an entry restores.
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
