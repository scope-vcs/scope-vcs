import { createIsomorphicFn } from '@tanstack/react-start'
import { getCookie } from '@tanstack/react-start/server'

const COOKIE = 'scope-requests-sidebar'

export const readRequestWorkspaceCollapsed = createIsomorphicFn()
  .server(() => getCookie(COOKIE) === 'collapsed')
  .client(() => document.cookie.split('; ').includes(`${COOKIE}=collapsed`))

export function saveRequestWorkspaceCollapsed(collapsed: boolean) {
  document.cookie = `${COOKIE}=${collapsed ? 'collapsed' : 'pinned'}; path=/; max-age=31536000; samesite=lax`
}
