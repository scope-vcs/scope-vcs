import type { GitHubAuthorizeResponse } from '../../api/types.generated'
import { storeSessionValue } from '../../lib/session-storage'
import { GITHUB_RETURN_PATH_KEY } from './github-setup-model'

/**
 * Sends a maintainer to GitHub to connect the repository, from the CI
 * settings or the Runs page alike. The page they leave is remembered, so the
 * setup flow returns there once the repository is connected. Resolves only if
 * GitHub's screen did not replace the page.
 */
export async function openGitHubAuthorization(start: () => Promise<GitHubAuthorizeResponse>) {
  const { authorize_url } = await start()
  storeSessionValue(GITHUB_RETURN_PATH_KEY, window.location.pathname)
  window.location.assign(authorize_url)
}
