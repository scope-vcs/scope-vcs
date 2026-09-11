import {
  createRepoInviteForRequest,
  deleteRepoInviteForRequest,
  deleteRepoMemberForRequest,
  deleteRepoForRequest,
  loadRepoCollaborationForRequest,
  parseCreateRepoInviteInput,
  parseDeleteRepoInviteInput,
  parseDeleteRepoMemberInput,
  parseRepoParams,
  parseUpdateRepoMemberInput,
  updateRepoMemberForRequest,
  updateRepoMetadataForRequest,
  parseUpdateRepoMetadataInput,
} from '@/api/repos'
import { HttpError } from '@/api/client'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { RepoSettingsPending } from '@/features/repo-detail/repo-settings-pending'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute } from '@tanstack/react-router'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback } from 'react'
import { getRequest } from '@tanstack/react-start/server'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { repoCollaborationResource, retainCollaborationResult } from '@/features/repo-detail/repo-collaboration-resource'
import type { CollaborationResult } from '@/features/repo-detail/repo-collaboration-results'
import { useCachedResource } from '@/lib/use-cached-resource'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { createServerFn } from '@tanstack/react-start'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    try {
      return await loadRepoCollaborationForRequest(data, getRequest().signal)
    } catch (error) {
      if (error instanceof HttpError && [403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

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

const deleteRepoInvite = createServerFn({ method: 'POST' })
  .validator(parseDeleteRepoInviteInput)
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

  async function retainResult<T>(mutation: Promise<T>, toChange: (value: T) => CollaborationResult) {
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
      {resource.value ? (
        <RepoSettingsPage
          createInvite={(data) => retainResult(createRepoInvite({ data }), ({ invite }) => ({ type: 'inviteUpdated', invite }))}
          deleteInvite={(data) => retainResult(deleteRepoInvite({ data }), (invite) => ({ type: 'inviteUpdated', invite }))}
          deleteRepo={(data) => deleteRepo({ data })}
          deleteMember={(data) => retainResult(deleteRepoMember({ data }), (member) => ({ type: 'memberRemoved', member }))}
          collaboration={resource.value.collaboration}
          params={params}
          updateMember={(data) => retainResult(updateRepoMember({ data }), (member) => ({ type: 'memberUpdated', member }))}
          updateMetadata={(data) => updateRepoMetadata({ data })}
        />
      ) : !resource.error ? <RepoSettingsPending /> : null}
    </>
  )
}
