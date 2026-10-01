import {
  completeGitHubSetupForRequest,
  connectRepoGitHubForRequest,
} from '@/api/github'
import { parseConnectRepoGitHubInput, parseGitHubSetupInput } from '@/api/github-inputs'
import { GitHubSetupView, type GitHubSetupSearch } from '@/features/github/github-setup-view'
import { invalidateRepoSettings } from '@/features/repo-detail/repo-settings-resource'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const completeGitHubSetup = createServerFn({ method: 'POST' })
  .validator(parseGitHubSetupInput)
  .handler(({ data }) => completeGitHubSetupForRequest(data))

const connectRepoGitHub = createServerFn({ method: 'POST' })
  .validator(parseConnectRepoGitHubInput)
  .handler(({ data }) => connectRepoGitHubForRequest(data))

// GitHub returns here from its install screen. Its Callback URL and Setup URL
// both point at this path.
export const Route = createFileRoute('/github/setup')({
  validateSearch: (search: Record<string, unknown>): GitHubSetupSearch => ({
    code: text(search.code),
    installation_id: positiveId(search.installation_id),
    setup_action: text(search.setup_action),
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
      search={search}
    />
  )
}

function text(value: unknown) {
  return typeof value === 'string' && value ? value : undefined
}

function positiveId(value: unknown) {
  const id = typeof value === 'string' ? Number(value) : value
  return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? id : undefined
}
