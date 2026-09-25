import type {
  CreateRepoInviteInput,
  RepoInviteInput,
  DeleteRepoMemberInput,
  RepoParams,
  UpdateRepoMemberInput,
  UpdateRepoMetadataInput,
} from '@/api/types'
import type {
  DeleteRepoResponse,
  RepositoryCollaborationResponse,
  RepositoryInviteLinkResponse,
  RepositoryInviteResponse,
  RepositoryMemberResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { storeHomeFlash } from '@/lib/home-flash'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { ShieldCheck } from 'lucide-react'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useReducer, useState } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  RepositoryMembersSection,
} from './repo-members-section'
import { MemberAccessSummary } from './repo-member-permissions'
import { RepositoryMetadataForm } from './repository-metadata-form'
import { AccessSection, DangerZoneSection } from './repo-settings-sections'
import { useRepoLayout } from './repo-layout-context'
import {
  initialRepoSettingsPageState,
  repoSettingsPageReducer,
} from './repo-settings-state'
import { mutateSettings } from './settings-mutation'

export function RepoSettingsPage({
  createInvite,
  createInviteLink,
  deleteInvite,
  sendInviteEmail,
  deleteMember,
  collaboration,
  collaborationLoading,
  deleteRepo,
  params,
  updateMember,
  updateMetadata,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<RepositoryInviteResponse>
  createInviteLink: (input: RepoInviteInput) => Promise<RepositoryInviteLinkResponse>
  deleteInvite: (input: RepoInviteInput) => Promise<RepositoryInviteResponse>
  sendInviteEmail: (input: RepoInviteInput) => Promise<RepositoryInviteResponse>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepositoryMemberResponse>
  collaboration: RepositoryCollaborationResponse | null
  /** The member list is still loading; the rest of the page does not need it. */
  collaborationLoading: boolean
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepositoryMemberResponse>
  updateMetadata: (input: UpdateRepoMetadataInput) => Promise<RepoSummaryResponse>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const { repo } = useRepoLayout()
  const [state, dispatch] = useReducer(
    repoSettingsPageReducer,
    initialRepoSettingsPageState,
  )
  const { deleteError, deleteTarget } = state
  const [refreshError, setRefreshError] = useState<string | null>(null)

  async function mutateAndRefresh<T>(mutation: Promise<T>) {
    setRefreshError(null)
    return mutateSettings(mutation, () => router.invalidate(), () => {
      setRefreshError('Your change was saved, but the updated settings could not be loaded. Refresh to try again.')
    })
  }

  async function deleteRepository(target: RepoSummaryResponse) {
    dispatch({ repo: target, type: 'deleteStarted' })
    try {
      await deleteRepo({
        owner: target.owner_handle,
        repo: target.name,
      })
      storeHomeFlash(`${target.id} deleted.`)
      await navigate({ to: '/' })
      void router.invalidate().catch(() => undefined)
    } catch (error) {
      dispatch({
        message: resourceErrorMessage(error, 'Repository deletion failed.'),
        type: 'deleteFailed',
      })
      throw error
    }
  }

  async function createMemberInvite(input: CreateRepoInviteInput) {
    return mutateAndRefresh(createInvite(input))
  }

  async function updateRepositoryMember(input: UpdateRepoMemberInput) {
    return mutateAndRefresh(updateMember(input))
  }

  async function removeRepositoryMember(memberUserId: string) {
    return mutateAndRefresh(
      deleteMember({ ...params, member_user_id: memberUserId }),
    )
  }

  async function removeRepositoryInvite(inviteId: string) {
    return mutateAndRefresh(deleteInvite({ ...params, invite_id: inviteId }))
  }

  return (
    <>
      <PageContent>
        <h1 className="sr-only">Settings</h1>
        {refreshError && (
          <PageErrorAlert title="Settings refresh failed">
            {refreshError}
          </PageErrorAlert>
        )}
        {deleteError && !deleteTarget && (
          <PageErrorAlert title="Repository deletion failed">
            {deleteError}
          </PageErrorAlert>
        )}

        {repo.access.actor === 'Public' && (
          <PageErrorAlert title="Settings unavailable">
            Sign in as the owner or a repository member to view repository
            access.
          </PageErrorAlert>
        )}

        {repo.access.actor !== 'Public' && (
          <RepositoryMetadataForm
            key={repo.id}
            repo={repo}
            save={(metadata) => mutateAndRefresh(updateMetadata({ ...params, ...metadata }))}
          />
        )}

        {repo.access.actor === 'Owner' && (
          <DangerZoneSection onDelete={() => dispatch({ repo, type: 'deleteTargetChanged' })} />
        )}

        {repo.access.actor === 'Member' && (
          <SectionRows>
            <SectionRow
              description="These permissions are assigned by the repository owner."
              icon={<ShieldCheck className="size-4" />}
              title="Your access"
            >
              <MemberAccessSummary permissions={repo.access} />
            </SectionRow>
          </SectionRows>
        )}

        {!collaboration && collaborationLoading && repo.access.actor === 'Owner' && (
          <AccessSection canInvite={repo.lifecycle_state === 'Ready'} ownerHandle={repo.owner_handle} />
        )}

        {collaboration && (
          <RepositoryMembersSection
            collaboration={collaboration}
            createInvite={createMemberInvite}
            createInviteLink={(inviteId) =>
              createInviteLink({ ...params, invite_id: inviteId })}
            deleteInvite={removeRepositoryInvite}
            sendInviteEmail={(inviteId) =>
              mutateAndRefresh(sendInviteEmail({ ...params, invite_id: inviteId }))}
            deleteMember={removeRepositoryMember}
            params={params}
            repo={repo}
            updateMember={updateRepositoryMember}
          />
        )}
      </PageContent>

      {deleteTarget && (
        <DeleteRepositoryDialog
          error={deleteError}
          onCancel={() =>
            dispatch({ repo: null, type: 'deleteTargetChanged' })
          }
          onConfirm={deleteRepository}
          repo={deleteTarget}
        />
      )}
    </>
  )
}
