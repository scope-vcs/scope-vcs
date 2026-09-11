import {
  parseCreateRepoInviteInput,
  parseDeleteRepoInviteInput,
  parseDeleteRepoMemberInput,
  parseUpdateRepoMemberInput,
  parseUpdateRepoMetadataInput,
} from '@/api/repo-inputs'
import { parseRepoParams } from '@/api/repo-params'
import {
  createRepoInviteForRequest,
  deleteRepoInviteForRequest,
  deleteRepoMemberForRequest,
  deleteRepoForRequest,
  loadRepoCollaborationForRequest,
  updateRepoMemberForRequest,
  updateRepoMetadataForRequest,
} from '@/api/repo-settings'
import { loadOptionalResource } from '@/api/http'
import { RepoSettingsPage } from '@/features/repo-detail/repo-settings-page'
import { RepoSettingsPending } from '@/features/repo-detail/repo-settings-pending'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoSettings = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadOptionalResource(() => loadRepoCollaborationForRequest(data)))

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
  loader: ({ params }) => loadRepoSettings({ data: params }),
  errorComponent: RepoContentError,
  pendingComponent: RepoSettingsPending,
  component: RepoSettingsRoute,
})

function RepoSettingsRoute() {
  const params = Route.useParams()
  const collaboration = Route.useLoaderData()
  return (
    <RepoSettingsPage
      createInvite={(data) => createRepoInvite({ data })}
      deleteInvite={(data) => deleteRepoInvite({ data })}
      deleteRepo={(data) => deleteRepo({ data })}
      deleteMember={(data) => deleteRepoMember({ data })}
      initialCollaboration={collaboration}
      params={params}
      updateMember={(data) => updateRepoMember({ data })}
      updateMetadata={(data) => updateRepoMetadata({ data })}
    />
  )
}
