import type {
  CreateRepoInviteInput,
  RepoParams,
  UpdateRepoMemberInput,
} from '@/api/types'
import type {
  RepositoryCollaborationResponse,
  RepositoryInviteLinkResponse,
  RepositoryInviteResponse,
  RepositoryMemberResponse,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { useState } from 'react'
import { RemovableRowList, RemoveButton } from './removable-row-list'
import { InviteMemberDialog } from './repo-invite-dialog'
import { InvitationList } from './repo-invite-list'
import { visibleInvitations } from './repo-invite-model'
import { AccessSection } from './repo-settings-sections'
import { AlwaysOnPrivateRead, PermissionEditor } from './repo-member-permissions'

/** One list for everyone with access or an invitation to it. */
export function RepositoryMembersSection({
  collaboration,
  createInvite,
  createInviteLink,
  deleteInvite,
  deleteMember,
  params,
  repo,
  sendInviteEmail,
  updateMember,
}: {
  collaboration: RepositoryCollaborationResponse
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<RepositoryInviteResponse>
  createInviteLink: (inviteId: string) => Promise<RepositoryInviteLinkResponse>
  deleteInvite: (inviteId: string) => Promise<RepositoryInviteResponse>
  deleteMember: (memberUserId: string) => Promise<RepositoryMemberResponse>
  params: RepoParams
  repo: RepoSummaryResponse
  sendInviteEmail: (inviteId: string) => Promise<RepositoryInviteResponse>
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepositoryMemberResponse>
}) {
  const [inviting, setInviting] = useState(false)
  const canInvite = repo.lifecycle_state === 'Ready'
  const invitations = visibleInvitations(
    collaboration.invites,
    collaboration.members.map((member) => member.email),
  )
  const invite = (input: Omit<CreateRepoInviteInput, 'owner' | 'repo'>) =>
    createInvite({ ...input, owner: params.owner, repo: params.repo })

  return (
    <AccessSection
      canInvite={canInvite}
      onInvite={() => setInviting(true)}
      ownerHandle={repo.owner_handle}
    >
      {collaboration.members.length > 0 && (
        <div className="border-t border-border pt-4">
          <MemberList
            deleteMember={deleteMember}
            members={collaboration.members}
            params={params}
            updateMember={updateMember}
          />
        </div>
      )}

      {invitations.length > 0 && (
        <div className="border-t border-border pt-4">
          <InvitationList
            createInviteLink={createInviteLink}
            deleteInvite={deleteInvite}
            invites={invitations}
            sendInviteEmail={sendInviteEmail}
            sendNewInvitation={invite}
          />
        </div>
      )}

      <InviteMemberDialog
        createInvite={invite}
        onOpenChange={setInviting}
        open={inviting}
        repoLabel={`${params.owner}/${params.repo}`}
      />
    </AccessSection>
  )
}

function MemberList({
  deleteMember,
  members,
  params,
  updateMember,
}: {
  deleteMember: (memberUserId: string) => Promise<RepositoryMemberResponse>
  members: RepositoryMemberResponse[]
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepositoryMemberResponse>
}) {
  return (
    <RemovableRowList
      confirm={{
        confirmLabel: 'Remove member',
        description: 'This immediately removes repository access for this member.',
        subject: (member) => `@${member.handle} · ${member.email}`,
        title: 'Remove repository member?',
      }}
      fallbackError="Member update failed."
      itemId={(member) => member.user_id}
      items={members}
      onRemove={(member) => deleteMember(member.user_id)}
      row={(member, actions) => (
        <>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="truncate text-sm font-medium leading-5">
                @{member.handle}
              </div>
              <div className="truncate text-sm leading-5 text-muted-foreground">
                {member.email}
              </div>
            </div>
            <RemoveButton label="Remove" onClick={actions.remove} pending={actions.pending} />
          </div>
          <AlwaysOnPrivateRead />
          <PermissionEditor
            disabled={actions.pending}
            onChange={(permissions) =>
              actions.run(() => updateMember({
                ...params,
                member_user_id: member.user_id,
                permissions,
              }))}
            permissions={member.permissions}
          />
        </>
      )}
      rowClassName="space-y-3 py-3 first:pt-0"
    />
  )
}
