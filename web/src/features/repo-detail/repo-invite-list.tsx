import type {
  RepositoryInviteLinkResponse,
  RepositoryInviteResponse,
  RepositoryMemberPermissions,
} from '@/api/types.generated'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link2, LoaderCircle, Send } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { RemovableRowList, RemoveButton, type RowActions } from './removable-row-list'
import { emailActionLabel, invitationDetail } from './repo-invite-model'
import { permissionSummaryText } from './repo-member-permission-model'

export function InvitationList({
  createInviteLink,
  deleteInvite,
  invites,
  sendInviteEmail,
  sendNewInvitation,
}: {
  createInviteLink: (inviteId: string) => Promise<RepositoryInviteLinkResponse>
  deleteInvite: (inviteId: string) => Promise<RepositoryInviteResponse>
  invites: RepositoryInviteResponse[]
  sendInviteEmail: (inviteId: string) => Promise<RepositoryInviteResponse>
  sendNewInvitation: (input: {
    email: string
    permissions: RepositoryMemberPermissions
  }) => Promise<RepositoryInviteResponse>
}) {
  // Holds a link only when the clipboard refused it, so it can be copied by hand.
  const [uncopiedLink, setUncopiedLink] = useState<string | null>(null)

  async function copyNewLink(inviteId: string) {
    setUncopiedLink(null)
    const { invite_url } = await createInviteLink(inviteId)
    try {
      await navigator.clipboard.writeText(invite_url)
      toast.success('New link copied. Earlier links still work.')
    } catch {
      setUncopiedLink(invite_url)
    }
  }

  async function sendEmail(invite: RepositoryInviteResponse) {
    await sendInviteEmail(invite.id)
    toast.success(`Invitation emailed to ${invite.invited_email}. Earlier links still work.`)
  }

  async function sendNew(invite: RepositoryInviteResponse) {
    await sendNewInvitation({
      email: invite.invited_email,
      permissions: invite.permissions,
    })
    toast.success(`New invitation sent to ${invite.invited_email}. The expired link stays inactive.`)
  }

  return (
    <>
      <RemovableRowList
        confirm={{
          confirmLabel: 'Revoke invitation',
          description: 'Every link for this invitation will stop working immediately. You can send a new invitation later.',
          subject: (invite) => invite.invited_email,
          title: 'Revoke this invitation?',
        }}
        fallbackError="Invitation update failed."
        itemId={(invite) => invite.id}
        items={invites}
        onRemove={(invite) => deleteInvite(invite.id)}
        row={(invite, actions) => (
          <>
            <div className="min-w-0">
              <div className="break-all font-medium leading-5">
                {invite.invited_email}
              </div>
              <div className="leading-5 text-muted-foreground">
                {permissionSummaryText(invite.permissions)}
              </div>
              <div className="leading-5 text-muted-foreground">
                {invitationDetail(invite)}
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={invite.state === 'Pending' ? 'warning' : 'neutral'}>
                {invite.state}
              </Badge>
              <InvitationActions
                actions={actions}
                copyNewLink={copyNewLink}
                invite={invite}
                sendEmail={sendEmail}
                sendNew={sendNew}
              />
            </div>
          </>
        )}
        rowClassName="flex flex-col gap-2 py-3 text-sm first:pt-0 lg:flex-row lg:items-center lg:justify-between"
      />
      {uncopiedLink && (
        <div className="mt-3 space-y-2">
          <p className="text-sm text-muted-foreground">
            The browser blocked the clipboard. Copy the new link from here.
          </p>
          <CopyableCodeBlock copyLabel="Copy invitation link" value={uncopiedLink} />
        </div>
      )}
    </>
  )
}

function InvitationActions({
  actions,
  copyNewLink,
  invite,
  sendEmail,
  sendNew,
}: {
  actions: RowActions
  copyNewLink: (inviteId: string) => Promise<void>
  invite: RepositoryInviteResponse
  sendEmail: (invite: RepositoryInviteResponse) => Promise<void>
  sendNew: (invite: RepositoryInviteResponse) => Promise<void>
}) {
  if (invite.state !== 'Pending') {
    return (
      <Button
        disabled={actions.pending}
        onClick={() => actions.run(() => sendNew(invite))}
        size="sm"
        type="button"
        variant="secondary"
      >
        {actions.pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <Send className="size-3.5" />}
        <span>Send new invitation</span>
      </Button>
    )
  }

  return (
    <>
      <Button
        disabled={actions.pending || invite.email?.state === 'queued'}
        onClick={() => actions.run(() => sendEmail(invite))}
        size="sm"
        type="button"
        variant="secondary"
      >
        <Send className="size-3.5" />
        <span>{emailActionLabel(invite)}</span>
      </Button>
      <Button
        disabled={actions.pending}
        onClick={() => actions.run(() => copyNewLink(invite.id))}
        size="sm"
        type="button"
        variant="secondary"
      >
        <Link2 className="size-3.5" />
        <span>Copy link</span>
      </Button>
      <RemoveButton label="Revoke" onClick={actions.remove} pending={actions.pending} />
    </>
  )
}
