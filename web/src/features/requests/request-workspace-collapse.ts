import { createIsomorphicFn } from '@tanstack/react-start'
import { getCookie } from '@tanstack/react-start/server'

const COOKIE = 'scope-requests-sidebar'

/**
 * Whether the viewer last left the requests sidebar collapsed. A cookie, not
 * local storage, so the server renders the sidebar the browser will keep.
 */
export const readRequestWorkspaceCollapsed = createIsomorphicFn()
  .server(() => getCookie(COOKIE) === 'collapsed')
  .client(() => document.cookie.split('; ').includes(`${COOKIE}=collapsed`))

export function saveRequestWorkspaceCollapsed(collapsed: boolean) {
  document.cookie = `${COOKIE}=${collapsed ? 'collapsed' : 'pinned'}; path=/; max-age=31536000; samesite=lax`
}
