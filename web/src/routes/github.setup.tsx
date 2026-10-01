import {
  completeGitHubSetupForRequest,
  connectRepoGitHubForRequest,
  startRepoGitHubAuthorizationForRequest,
} from '@/api/github'
import { parseConnectRepoGitHubInput, parseGitHubSetupInput } from '@/api/github-inputs'
import { parseRepoParams } from '@/api/repo-params'
import {
  encodePendingGitHubTarget,
  parsePendingGitHubTarget,
  PENDING_GITHUB_TARGET_KEY,
  type GitHubSetupSearch,
} from '@/features/github/github-setup-model'
import { GitHubSetupView } from '@/features/github/github-setup-view'
import { invalidateRepoSettings } from '@/features/repo-detail/repo-settings-resource'
import { readAndClearSessionValue, storeSessionValue } from '@/lib/session-storage'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const completeGitHubSetup = createServerFn({ method: 'POST' })
  .validator(parseGitHubSetupInput)
  .handler(({ data }) => completeGitHubSetupForRequest(data))

const connectRepoGitHub = createServerFn({ method: 'POST' })
  .validator(parseConnectRepoGitHubInput)
  .handler(({ data }) => connectRepoGitHubForRequest(data))

const startRepoGitHubAuthorization = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => startRepoGitHubAuthorizationForRequest(data))

// The app's Callback URL (after OAuth) and Setup URL (after installing) are
// both this path. Installation ids GitHub adds to the URL are never read.
export const Route = createFileRoute('/github/setup')({
  validateSearch: (search: Record<string, unknown>): GitHubSetupSearch => ({
    code: text(search.code),
    error: text(search.error),
    state: text(search.state),
  }),
  component: GitHubSetupRoute,
})

function GitHubSetupRoute() {
  const search = Route.useSearch()
  const navigate = useNavigate()
  return (
    <GitHubSetupView
      completeSetup={(data) => completeGitHubSetup({ data })}
      connect={(data) => connectRepoGitHub({ data })}
      onConnected={async (setup) => {
        invalidateRepoSettings(`${setup.owner_handle}/${setup.repo_name}`)
        await navigate({
          params: { owner: setup.owner_handle, repo: setup.repo_name },
          to: '/$owner/$repo/settings',
        })
      }}
      rememberPendingTarget={(target) =>
        storeSessionValue(PENDING_GITHUB_TARGET_KEY, encodePendingGitHubTarget(target))}
      search={search}
      startAuthorization={(data) => startRepoGitHubAuthorization({ data })}
      takePendingTarget={() =>
        parsePendingGitHubTarget(readAndClearSessionValue(PENDING_GITHUB_TARGET_KEY))}
    />
  )
}

function text(value: unknown) {
  return typeof value === 'string' && value ? value : undefined
}
