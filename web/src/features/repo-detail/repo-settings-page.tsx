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
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { storeHomeFlash } from '@/lib/home-flash'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { ShieldCheck, Trash2 } from 'lucide-react'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useReducer } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSummary,
  RepositoryMembersSection,
} from './repo-members-section'
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
  collaboration,
  deleteRepo,
  params,
  updateMember,
  updateMetadata,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepositoryInviteResponse>
  deleteInvite: (input: DeleteRepoInviteInput) => Promise<RepositoryInviteResponse>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepositoryMemberResponse>
  collaboration: RepositoryCollaborationResponse | null
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
          <SectionRows>
            <SectionRow
              description="Permanently removes repo metadata and stored Git data from Scope."
              icon={<Trash2 className="size-4" />}
              title="Danger zone"
            >
              <Button
                onClick={() => dispatch({ repo, type: 'deleteTargetChanged' })}
                size="sm"
                type="button"
                variant="destructive"
              >
                <Trash2 className="size-3.5" />
                <span>Delete repository</span>
              </Button>
            </SectionRow>
          </SectionRows>
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
