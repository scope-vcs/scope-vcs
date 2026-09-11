import { mutateSettings } from './settings-mutation'
import type {
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  DeleteRepoResponse,
  RepoCollaboration,
  RepoInvite,
  RepoMember,
  RepoParams,
  RepoSummary,
  UpdateRepoMemberInput,
  UpdateRepoMetadataInput,
} from '@/api/types'
import { PageContent } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { storeHomeFlash } from '@/lib/home-flash'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useState } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSections,
  RepositoryMembersSection,
} from './repo-members-section'
import { SettingsSections } from './repo-settings-sections'
import { RepositoryMetadataForm } from './repository-metadata-form'
import { useRepoLayout } from './repo-layout-context'

export function RepoSettingsPage({
  createInvite,
  deleteInvite,
  deleteMember,
  deleteRepo,
  collaboration,
  params,
  updateMember,
  updateMetadata,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepoInviteResponse>
  deleteInvite: (input: DeleteRepoInviteInput) => Promise<RepoInvite>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepoMember>
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  collaboration: RepoCollaboration | null
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
  updateMetadata: (input: UpdateRepoMetadataInput) => Promise<RepoSummary>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const { repo } = useRepoLayout()
  const [deleteTarget, setDeleteTarget] = useState<RepoSummary | null>(null)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [refreshError, setRefreshError] = useState<string | null>(null)

  async function mutateAndRefresh<T>(mutation: Promise<T>) {
    setRefreshError(null)
    return mutateSettings(mutation, () => router.invalidate(), () => {
      setRefreshError('Your change was saved, but the updated settings could not be loaded. Refresh to try again.')
    })
  }

  async function deleteRepository(target: RepoSummary) {
    setDeleteTarget(target)
    setDeleteError(null)
    try {
      await deleteRepo({
        owner: target.owner_handle,
        repo: target.name,
      })
      storeHomeFlash(`${target.id} deleted.`)
      await navigate({ to: '/' })
      void router.invalidate().catch(() => undefined)
    } catch (error) {
      setDeleteError(error instanceof Error ? error.message : 'repository deletion failed')
      throw error
    }
  }

  return (
    <>
      <PageContent>
        <h1 className="sr-only">Settings</h1>
        {refreshError && <PageErrorAlert title="Settings refresh failed">{refreshError}</PageErrorAlert>}
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
          <SettingsSections
            onDeleteRepository={() =>
              setDeleteTarget(repo)
            }
          />
        )}

        {repo.access.actor === 'Member' && (
          <MemberAccessSections repo={repo} />
        )}

        {collaboration && (
          <RepositoryMembersSection
            collaboration={collaboration}
            createInvite={input => mutateAndRefresh(createInvite(input))}
            deleteInvite={invite_id => mutateAndRefresh(deleteInvite({ ...params, invite_id }))}
            deleteMember={member_user_id => mutateAndRefresh(deleteMember({ ...params, member_user_id }))}
            params={params}
            repo={repo}
            updateMember={input => mutateAndRefresh(updateMember(input))}
          />
        )}
      </PageContent>

      {deleteTarget && (
        <DeleteRepositoryDialog
          error={deleteError}
          onCancel={() =>
            setDeleteTarget(null)
          }
          onConfirm={deleteRepository}
          repo={deleteTarget}
        />
      )}
    </>
  )
}
