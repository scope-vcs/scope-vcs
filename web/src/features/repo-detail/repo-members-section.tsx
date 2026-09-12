import type {
  CreateRepoInviteInput,
  RepoParams,
  UpdateRepoMemberInput,
} from '@/api/types'
import type {
  CreateRepositoryInviteResponse,
  RepositoryCollaborationResponse,
  RepositoryInviteResponse,
  RepositoryMemberResponse,
  RepositoryMemberPermissions,
  RepoSummaryResponse,
} from '@/api/types.generated'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import {
  Eye,
  LoaderCircle,
  MailPlus,
  Trash2,
  Users,
} from 'lucide-react'
import { useReducer, useState, type FormEvent, type ReactNode } from 'react'

const defaultPermissions: RepositoryMemberPermissions = {
  can_apply_changes: false,
  can_change_file_visibility: false,
  can_push: false,
}

// The API carries three permissions; push is the only one members can hold today.
const PUSH_PERMISSION = {
  description: 'Allows Git pushes to this repository.',
  label: 'Push changes',
}

type InviteMemberFormState = {
  email: string
  error: string | null
  inviteUrl: string | null
  pending: boolean
  permissions: RepositoryMemberPermissions
}

type InviteMemberFormAction =
  | { email: string; type: 'emailChanged' }
  | { permissions: RepositoryMemberPermissions; type: 'permissionsChanged' }
  | { type: 'submitStarted' }
  | { inviteUrl: string; type: 'submitSucceeded' }
  | { message: string; type: 'submitFailed' }

const initialInviteMemberFormState: InviteMemberFormState = {
  email: '',
  error: null,
  inviteUrl: null,
  pending: false,
  permissions: defaultPermissions,
}

function inviteMemberFormReducer(
  state: InviteMemberFormState,
  action: InviteMemberFormAction,
): InviteMemberFormState {
  switch (action.type) {
    case 'emailChanged':
      return { ...state, email: action.email }
    case 'permissionsChanged':
      return { ...state, permissions: action.permissions }
    case 'submitStarted':
      return { ...state, error: null, inviteUrl: null, pending: true }
    case 'submitSucceeded':
      return {
        ...state,
        email: '',
        inviteUrl: action.inviteUrl,
        pending: false,
        permissions: defaultPermissions,
      }
    case 'submitFailed':
      return { ...state, error: action.message, pending: false }
  }
}

/** What a member can do: private read is always on, push is the one toggle. */
export function MemberAccessSummary({
  permissions,
}: {
  permissions: RepositoryMemberPermissions
}) {
  return (
    <div className="space-y-3 text-sm">
      <AlwaysOnPrivateRead />
      <div className="flex items-center justify-between gap-3">
        <span>{PUSH_PERMISSION.label}</span>
        <Badge variant={permissions.can_push ? 'success' : 'neutral'}>
          {permissions.can_push ? 'On' : 'Off'}
        </Badge>
      </div>
    </div>
  )
}

export function RepositoryMembersSection({
  collaboration,
  createInvite,
  deleteInvite,
  deleteMember,
  params,
  repo,
  updateMember,
}: {
  collaboration: RepositoryCollaborationResponse
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepositoryInviteResponse>
  deleteInvite: (inviteId: string) => Promise<RepositoryInviteResponse>
  deleteMember: (memberUserId: string) => Promise<RepositoryMemberResponse>
  params: RepoParams
  repo: RepoSummaryResponse
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepositoryMemberResponse>
}) {
  const canInvite = repo.lifecycle_state === 'Ready'
  const pendingInvites = collaboration.invites.filter(
    (invite) => invite.state === 'Pending',
  )

  return (
    <SectionRows>
      <SectionRow
        description={
          canInvite
            ? 'Invite members by email and assign only the extra actions they need.'
            : 'Members can be invited after the first Scope push is applied.'
        }
        icon={<MailPlus className="size-4" />}
        title="Invite member"
      >
        <InviteMemberForm
          canInvite={canInvite}
          createInvite={(input) =>
            createInvite({
              ...input,
              owner: params.owner,
              repo: params.repo,
            })
          }
        />
      </SectionRow>

      <SectionRow
        description="Members always read private files. Toggles only control repository actions."
        icon={<Users className="size-4" />}
        title="Members"
      >
        <MemberList
          deleteMember={deleteMember}
          members={collaboration.members}
          params={params}
          updateMember={updateMember}
        />
      </SectionRow>

      {pendingInvites.length > 0 && (
        <SectionRow
          description="Pending email invites are unique per repository and email."
          icon={<MailPlus className="size-4" />}
          title="Pending invites"
        >
          <InviteList deleteInvite={deleteInvite} invites={pendingInvites} />
        </SectionRow>
      )}
    </SectionRows>
  )
}

function InviteMemberForm({
  canInvite,
  createInvite,
}: {
  canInvite: boolean
  createInvite: (
    input: Omit<CreateRepoInviteInput, 'owner' | 'repo'>,
  ) => Promise<CreateRepositoryInviteResponse>
}) {
  const [state, dispatch] = useReducer(
    inviteMemberFormReducer,
    initialInviteMemberFormState,
  )

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!canInvite || state.pending) {
      return
    }

    dispatch({ type: 'submitStarted' })
    try {
      const response = await createInvite({
        email: state.email,
        permissions: state.permissions,
      })
      dispatch({ inviteUrl: response.invite_url, type: 'submitSucceeded' })
    } catch (error) {
      dispatch({
        message: resourceErrorMessage(error, 'Invite could not be created.'),
        type: 'submitFailed',
      })
    }
  }

  return (
    <form className="space-y-4" onSubmit={(event) => void submit(event)}>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Input
          aria-label="Member email"
          disabled={!canInvite || state.pending}
          onChange={(event) =>
            dispatch({ email: event.target.value, type: 'emailChanged' })
          }
          placeholder="teammate@example.com"
          type="email"
          value={state.email}
        />
        <Button
          disabled={!canInvite || state.pending || !state.email.trim()}
          type="submit"
        >
          {state.pending ? (
            <LoaderCircle className="size-3.5 animate-spin" />
          ) : (
            <MailPlus className="size-3.5" />
          )}
          <span>Invite</span>
        </Button>
      </div>

      <div className="rounded-md border border-warning-border bg-warning-soft px-3 py-2 text-sm leading-5 text-warning-strong">
        Members always read private files once they accept. This toggle grants
        repository push access only.
      </div>

      <PushPermissionToggle
        disabled={!canInvite || state.pending}
        onChange={(permissions) =>
          dispatch({ permissions, type: 'permissionsChanged' })
        }
        permissions={state.permissions}
      />

      {state.error && <p className="text-sm text-destructive" role="alert">{state.error}</p>}
      {state.inviteUrl && (
        <div className="space-y-2">
          <p className="text-sm text-muted-foreground">
            Invite created. Share this link with the invitee.
          </p>
          <CopyableCodeBlock
            copyLabel="Copy invite link"
            value={state.inviteUrl}
          />
        </div>
      )}
    </form>
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
  if (members.length === 0) {
    return (
      <p className="text-sm leading-5 text-muted-foreground">
        No members have accepted an invite yet.
      </p>
    )
  }

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
          <PushPermissionToggle
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

function InviteList({
  deleteInvite,
  invites,
}: {
  deleteInvite: (inviteId: string) => Promise<RepositoryInviteResponse>
  invites: RepositoryInviteResponse[]
}) {
  return (
    <RemovableRowList
      confirm={{
        confirmLabel: 'Revoke invite',
        description: 'The current invite link will stop working immediately.',
        subject: (invite) => invite.invited_email,
        title: 'Revoke pending invite?',
      }}
      fallbackError="Invite revoke failed."
      itemId={(invite) => invite.id}
      items={invites}
      onRemove={(invite) => deleteInvite(invite.id)}
      row={(invite, actions) => (
        <>
          <div className="min-w-0">
            <div className="truncate font-medium leading-5">
              {invite.invited_email}
            </div>
            <div className="leading-5 text-muted-foreground">
              {invite.permissions.can_push ? 'push changes' : 'No extra actions'}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="warning">{invite.state}</Badge>
            <RemoveButton label="Revoke" onClick={actions.remove} pending={actions.pending} />
          </div>
        </>
      )}
      rowClassName="flex flex-col gap-2 py-3 text-sm first:pt-0 sm:flex-row sm:items-center sm:justify-between"
    />
  )
}

type RowActions = {
  pending: boolean
  remove: () => void
  run: (action: () => Promise<unknown>) => void
}

/**
 * A list whose rows can run one async action at a time and be removed after a
 * confirmation. Owns the shared pending, error, and confirm-target state so
 * member and invite rows cannot drift in how they report a failed call.
 */
function RemovableRowList<Item>({
  confirm,
  fallbackError,
  itemId,
  items,
  onRemove,
  row,
  rowClassName,
}: {
  confirm: {
    confirmLabel: string
    description: string
    subject: (item: Item) => string
    title: string
  }
  fallbackError: string
  itemId: (item: Item) => string
  items: readonly Item[]
  onRemove: (item: Item) => Promise<unknown>
  row: (item: Item, actions: RowActions) => ReactNode
  rowClassName: string
}) {
  const [error, setError] = useState<string | null>(null)
  const [confirmTarget, setConfirmTarget] = useState<Item | null>(null)
  const [pendingId, setPendingId] = useState<string | null>(null)

  async function run(item: Item, action: () => Promise<unknown>) {
    setError(null)
    setPendingId(itemId(item))
    try {
      await action()
    } catch (error) {
      setError(resourceErrorMessage(error, fallbackError))
    } finally {
      setPendingId(null)
    }
  }

  async function remove(item: Item) {
    try {
      await run(item, () => onRemove(item))
    } finally {
      setConfirmTarget(null)
    }
  }

  return (
    <div className="space-y-3">
      <ul className="divide-y divide-border">
        {items.map((item) => {
          const id = itemId(item)
          return (
            <li className={rowClassName} key={id}>
              {row(item, {
                pending: pendingId === id,
                remove: () => setConfirmTarget(item),
                run: (action) => void run(item, action),
              })}
            </li>
          )
        })}
      </ul>
      <DestructiveActionDialog
        confirmLabel={confirm.confirmLabel}
        description={confirm.description}
        onConfirm={() => {
          if (confirmTarget !== null) void remove(confirmTarget)
        }}
        onOpenChange={(open) => {
          if (!open && !pendingId) setConfirmTarget(null)
        }}
        open={confirmTarget !== null}
        pending={confirmTarget !== null && pendingId === itemId(confirmTarget)}
        subject={confirmTarget !== null ? confirm.subject(confirmTarget) : ''}
        title={confirm.title}
      />
      {error && <p className="text-sm text-destructive" role="alert">{error}</p>}
    </div>
  )
}

function RemoveButton({
  label,
  onClick,
  pending,
}: {
  label: string
  onClick: () => void
  pending: boolean
}) {
  return (
    <Button
      disabled={pending}
      onClick={onClick}
      size="sm"
      type="button"
      variant="secondary"
    >
      {pending ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : (
        <Trash2 className="size-3.5" />
      )}
      <span>{label}</span>
    </Button>
  )
}

function PushPermissionToggle({
  disabled,
  onChange,
  permissions,
}: {
  disabled?: boolean
  onChange: (permissions: RepositoryMemberPermissions) => void
  permissions: RepositoryMemberPermissions
}) {
  return (
    <label className="flex items-start justify-between gap-4 text-sm">
      <span className="min-w-0">
        <span className="block font-medium leading-5">{PUSH_PERMISSION.label}</span>
        <span className="block leading-5 text-muted-foreground">
          {PUSH_PERMISSION.description}
        </span>
      </span>
      <Switch
        checked={permissions.can_push}
        disabled={disabled}
        onCheckedChange={(checked) => onChange({ ...permissions, can_push: checked })}
        type="button"
      />
    </label>
  )
}

function AlwaysOnPrivateRead() {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="inline-flex items-center gap-2">
        <Eye className="size-3.5 text-muted-foreground" />
        <span>Read private files</span>
      </span>
      <Badge variant="success">Always on</Badge>
    </div>
  )
}
