import {
  parseCreateRepoInviteInput,
  parseRepoInviteInput,
  parseDeleteRepoMemberInput,
  parseUpdateRepoMemberInput,
  parseUpdateRepoMetadataInput,
} from '@/api/repo-inputs'
import { parseRepoParams } from '@/api/repo-params'
import {
  createRepoInviteForRequest,
  createRepoInviteLinkForRequest,
  deleteRepoInviteForRequest,
  deleteRepoMemberForRequest,
  deleteRepoForRequest,
  sendRepoInviteEmailForRequest,
  updateRepoMemberForRequest,
  updateRepoMetadataForRequest,
} from '@/api/repo-settings'
import {
  confirmRepoGitHubPublicForRequest,
  disconnectRepoGitHubForRequest,
  setRepoGitHubMainPublicationForRequest,
  setRepoGitHubRequiredChecksForRequest,
  setRepoGitHubRunImportCountForRequest,
  startRepoGitHubRunImportForRequest,
  startRepoGitHubSetupCheckForRequest,
} from '@/api/github'
import {
  parseSetRepoGitHubMainPublicationInput,
  parseSetRepoGitHubRequiredChecksInput,
  parseSetRepoGitHubRunImportCountInput,
} from '@/api/github-inputs'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { VisibilityLogSection } from '@/features/repo-detail/visibility-log-section'
import { RepositoryDependencySection } from '@/features/repo-detail/repository-dependency-check'
import { RepoSettingsPending } from '@/features/repo-detail/repo-settings-pending'
import { RepoContentError } from '@/components/repo-content-error'
import { WorkbenchBar, WorkbenchBody, WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import type { RepoLiveState } from '@/api/types'
import { useAuth } from '@clerk/tanstack-react-start'
import { auth } from '@clerk/tanstack-react-start/server'
import { useCallback, useEffect } from 'react'
import { useRepoLayout, useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import {
  refreshWhenNextInviteExpires,
  repoSettingsResource,
  retainCollaborationResult,
  retainGitHubConnection,
} from '@/features/repo-detail/repo-settings-resource'
import type { CollaborationResult } from '@/features/repo-detail/repo-collaboration-results'
import { useCachedResource } from '@/lib/use-cached-resource'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { loadRepoSettingsData, startRepoGitHubAuthorization } from './-repo-settings-actions'

const startRepoGitHubSetupCheck = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => startRepoGitHubSetupCheckForRequest(data))

const disconnectRepoGitHub = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => disconnectRepoGitHubForRequest(data))

const confirmRepoGitHubPublic = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => confirmRepoGitHubPublicForRequest(data))

const setRepoGitHubMainPublication = createServerFn({ method: 'POST' })
  .validator(parseSetRepoGitHubMainPublicationInput)
  .handler(({ data }) => setRepoGitHubMainPublicationForRequest(data))

const setRepoGitHubRequiredChecks = createServerFn({ method: 'POST' })
  .validator(parseSetRepoGitHubRequiredChecksInput)
  .handler(({ data }) => setRepoGitHubRequiredChecksForRequest(data))

const setRepoGitHubRunImportCount = createServerFn({ method: 'POST' })
  .validator(parseSetRepoGitHubRunImportCountInput)
  .handler(({ data }) => setRepoGitHubRunImportCountForRequest(data))

const startRepoGitHubRunImport = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => startRepoGitHubRunImportForRequest(data))

const deleteRepo = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => deleteRepoForRequest(data))

const createRepoInvite = createServerFn({ method: 'POST' })
  .validator(parseCreateRepoInviteInput)
  .handler(({ data }) => createRepoInviteForRequest(data))

const updateRepoMember = createServerFn({ method: 'POST' })
  .validator(parseUpdateRepoMemberInput)
  .handler(({ data }) => updateRepoMemberForRequest(data))

const updateRepoMetadata = createServerFn({ method: 'POST' })
  .validator(parseUpdateRepoMetadataInput)
  .handler(({ data }) => updateRepoMetadataForRequest(data))

const deleteRepoMember = createServerFn({ method: 'POST' })
  .validator(parseDeleteRepoMemberInput)
  .handler(({ data }) => deleteRepoMemberForRequest(data))

const createRepoInviteLink = createServerFn({ method: 'POST' })
  .validator(parseRepoInviteInput)
  .handler(({ data }) => createRepoInviteLinkForRequest(data))

const sendRepoInviteEmail = createServerFn({ method: 'POST' })
  .validator(parseRepoInviteInput)
  .handler(({ data }) => sendRepoInviteEmailForRequest(data))

const deleteRepoInvite = createServerFn({ method: 'POST' })
  .validator(parseRepoInviteInput)
  .handler(({ data }) => deleteRepoInviteForRequest(data))

export const Route = createFileRoute('/$owner/$repo/settings')({
  loader: async ({ params, parentMatchPromise, abortController }) => {
    if (typeof window !== 'undefined') return null
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { userId } = await auth()
    const settings = await loadRepoSettingsData(params, abortController.signal).catch(() => null)
    return settings && { scope: repoResourceScope(live.repo, userId), settings }
  },
  errorComponent: RepoContentError,
  pendingComponent: RepoSettingsPending,
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const initial = Route.useLoaderData()
  const { repo } = useRepoLayout()
  const views = useRepoViews()
  const { isLoaded, userId } = useAuth()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : initial?.scope ?? null
  const initialValue = initial?.scope === scope ? initial.settings : null
  const { owner, repo: repoName } = params
  const load = useCallback(
    (signal: AbortSignal) => loadRepoSettingsData({ owner, repo: repoName }, signal),
    [owner, repoName],
  )
  const resource = useCachedResource({
    identity: scope,
    resource: repoSettingsResource,
    initialValue,
    load,
    fallbackError: 'Repository settings could not be loaded.',
  })
  const settings = resource.value ?? initialValue

  const collaboration = settings?.collaboration ?? null
  useEffect(
    () => (scope ? refreshWhenNextInviteExpires(scope, collaboration) : undefined),
    [scope, collaboration],
  )

  async function retainResult<T>(
    mutation: Promise<T>,
    toChange: (value: T) => CollaborationResult,
  ) {
    const result = await mutation
    if (scope) retainCollaborationResult(scope, toChange(result))
    return result
  }

  return (
    <WorkbenchPane>
      <WorkbenchBar title="Settings" />
      <WorkbenchBody>
        {resource.error && (
          <>
            <PageErrorAlert title="Settings refresh failed">{resource.error}</PageErrorAlert>
            <Button className="mt-3" onClick={resource.retry} size="sm">Try again</Button>
          </>
        )}
        {!resource.error || settings ? (
          <RepoSettingsPage
            key={scope}
            dependencyCheck={<RepositoryDependencySection params={params} repo={repo} />}
            visibilityLog={views.full && <VisibilityLogSection params={params} view={views.full} />}
            createInvite={(data) => retainResult(
              createRepoInvite({ data }),
              (invite) => ({ type: 'inviteUpdated', invite }),
            )}
            createInviteLink={(data) => createRepoInviteLink({ data })}
            sendInviteEmail={(data) => retainResult(
              sendRepoInviteEmail({ data }),
              (invite) => ({ type: 'inviteUpdated', invite }),
            )}
            deleteInvite={(data) => retainResult(
              deleteRepoInvite({ data }),
              (invite) => ({ type: 'inviteUpdated', invite }),
            )}
            deleteRepo={(data) => deleteRepo({ data })}
            confirmPublicGitHub={async (data) => {
              const github = await confirmRepoGitHubPublic({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            disconnectGitHub={async (data) => {
              const github = await disconnectRepoGitHub({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            github={settings?.github ?? null}
            setGitHubMainPublication={async (data) => {
              const github = await setRepoGitHubMainPublication({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            setGitHubRequiredChecks={async (data) => {
              const github = await setRepoGitHubRequiredChecks({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            setGitHubRunImportCount={async (data) => {
              const github = await setRepoGitHubRunImportCount({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            startGitHubAuthorization={startRepoGitHubAuthorization}
            startGitHubRunImport={async (data) => {
              const github = await startRepoGitHubRunImport({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            startGitHubSetupCheck={async (data) => {
              const github = await startRepoGitHubSetupCheck({ data })
              if (scope) retainGitHubConnection(scope, github)
              return github
            }}
            deleteMember={(data) => retainResult(
              deleteRepoMember({ data }),
              (member) => ({ type: 'memberRemoved', member }),
            )}
            collaboration={collaboration}
            collaborationLoading={!settings}
            params={params}
            updateMember={(data) => retainResult(
              updateRepoMember({ data }),
              (member) => ({ type: 'memberUpdated', member }),
            )}
            updateMetadata={(data) => updateRepoMetadata({ data })}
          />
        ) : null}
      </WorkbenchBody>
    </WorkbenchPane>
  )
}
