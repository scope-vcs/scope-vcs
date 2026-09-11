import type {
  CreateRepoInviteInput,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  RepoParams,
  UpdateRepoMemberInput,
  UpdateRepoMetadataInput,
} from '@/api/types'
import type {
  CreateRepositoryInviteResponse,
  DeleteRepoResponse,
  RepositoryCollaborationResponse,
  RepositoryInviteResponse,
  RepositoryMemberResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { storeHomeFlash } from '@/lib/home-flash'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useReducer } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSections,
  RepositoryMembersSection,
} from './repo-members-section'
import { SettingsSections } from './repo-settings-sections'
import { RepositoryMetadataForm } from './repository-metadata-form'
import { useRepoLayout } from './repo-layout-context'
import {
  initialRepoSettingsPageState,
  repoSettingsPageReducer,
} from './repo-settings-state'

export function RepoSettingsPage({
  createInvite,
  deleteInvite,
  deleteMember,
  deleteRepo,
  initialCollaboration,
  params,
  updateMember,
  updateMetadata,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepositoryInviteResponse>
  deleteInvite: (input: DeleteRepoInviteInput) => Promise<RepositoryInviteResponse>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepositoryMemberResponse>
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  initialCollaboration: RepositoryCollaborationResponse | null
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
  const collaboration = initialCollaboration
  const { deleteError, deleteTarget } = state

  async function mutateAndRefresh<T>(mutation: Promise<T>) {
    const result = await mutation
    await router.invalidate()
    return result
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
        message:
          error instanceof Error ? error.message : 'repository deletion failed',
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
        {deleteError && (
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
            repo={repo}
            save={(metadata) => mutateAndRefresh(updateMetadata({ ...params, ...metadata }))}
          />
        )}

        {repo.access.actor === 'Owner' && (
          <SettingsSections
            onDeleteRepository={() =>
              dispatch({ repo, type: 'deleteTargetChanged' })
            }
          />
        )}

        {repo.access.actor === 'Member' && (
          <MemberAccessSections repo={repo} />
        )}

        {collaboration && (
          <RepositoryMembersSection
            collaboration={collaboration}
            createInvite={createMemberInvite}
            deleteInvite={removeRepositoryInvite}
            deleteMember={removeRepositoryMember}
            params={params}
            repo={repo}
            updateMember={updateRepositoryMember}
          />
        )}
      </PageContent>

      {deleteTarget && (
        <DeleteRepositoryDialog
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
