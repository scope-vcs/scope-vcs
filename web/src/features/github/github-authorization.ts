import type { GitHubAuthorizeResponse } from '../../api/types.generated'
import { storeSessionValue } from '../../lib/session-storage'
import { GITHUB_RETURN_PATH_KEY } from './github-setup-model'

export async function openGitHubAuthorization(start: () => Promise<GitHubAuthorizeResponse>) {
  const { authorize_url } = await start()
  storeSessionValue(GITHUB_RETURN_PATH_KEY, window.location.pathname)
  window.location.assign(authorize_url)
}
