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
  loadRepoCollaborationForRequest,
  sendRepoInviteEmailForRequest,
  updateRepoMemberForRequest,
  updateRepoMetadataForRequest,
} from '@/api/repo-settings'
import { loadOptionalResource } from '@/api/http'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { VisibilityLogSection } from '@/features/repo-detail/visibility-log-section'
import { RepoSettingsPending } from '@/features/repo-detail/repo-settings-pending'
import { RepoContentError } from '@/components/repo-content-error'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect } from 'react'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import {
  refreshWhenNextInviteExpires,
  repoCollaborationResource,
  retainCollaborationResult,
} from '@/features/repo-detail/repo-collaboration-resource'
import type { CollaborationResult } from '@/features/repo-detail/repo-collaboration-results'
import { useCachedResource } from '@/lib/use-cached-resource'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadOptionalResource(() => loadRepoCollaborationForRequest(data, getRequest().signal)))

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
  errorComponent: RepoContentError,
  pendingComponent: RepoSettingsPending,
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const { isLoaded, userId } = useAuth()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const load = useCallback(async (signal: AbortSignal) => ({
    collaboration: await loadRepoSettings({ data: { owner, repo: repoName }, signal }),
  }), [owner, repoName])
  const resource = useCachedResource({
    identity: scope,
    resource: repoCollaborationResource,
    load,
    fallbackError: 'Repository access settings could not be loaded.',
  })

  const collaboration = resource.value?.collaboration ?? null
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
    <>
      {resource.error && (
        <PageContent>
          <PageErrorAlert title="Settings refresh failed">{resource.error}</PageErrorAlert>
          <Button className="mt-3" onClick={resource.retry} size="sm">Try again</Button>
        </PageContent>
      )}
      {!resource.error || resource.value ? (
        <RepoSettingsPage
          key={scope}
          visibilityLog={<VisibilityLogSection params={params} />}
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
          deleteMember={(data) => retainResult(
            deleteRepoMember({ data }),
            (member) => ({ type: 'memberRemoved', member }),
          )}
          collaboration={collaboration}
          collaborationLoading={!resource.value}
          params={params}
          updateMember={(data) => retainResult(
            updateRepoMember({ data }),
            (member) => ({ type: 'memberUpdated', member }),
          )}
          updateMetadata={(data) => updateRepoMetadata({ data })}
        />
      ) : null}
    </>
  )
}
