import type { CreateRepoInviteInput } from '@/api/types'
import type {
  RepositoryInviteResponse,
  RepositoryMemberPermissions,
} from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import * as Dialog from '@radix-ui/react-dialog'
import { LoaderCircle, Send } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { notEmailedNotice } from './repo-invite-model'
import { defaultPermissions } from './repo-member-permission-model'
import { PermissionEditor } from './repo-member-permissions'

export function InviteMemberDialog({
  createInvite,
  onOpenChange,
  open,
  repoLabel,
}: {
  createInvite: (
    input: Omit<CreateRepoInviteInput, 'owner' | 'repo'>,
  ) => Promise<RepositoryInviteResponse>
  onOpenChange: (open: boolean) => void
  open: boolean
  repoLabel: string
}) {
  const [email, setEmail] = useState('')
  const [permissions, setPermissions] =
    useState<RepositoryMemberPermissions>(defaultPermissions)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  function changeOpen(next: boolean) {
    if (pending) return
    if (!next) {
      setEmail('')
      setPermissions(defaultPermissions)
      setError(null)
    }
    onOpenChange(next)
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (pending) return

    setError(null)
    setPending(true)
    try {
      const created = await createInvite({ email, permissions })
      if (!created.email) toast.warning(notEmailedNotice)
      setPending(false)
      changeOpen(false)
    } catch (error) {
      setError(resourceErrorMessage(error, 'Invitation could not be sent.'))
      setPending(false)
    }
  }

  return (
    <Dialog.Root onOpenChange={changeOpen} open={open}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content className="fixed top-1/2 left-1/2 z-50 grid max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-[520px] -translate-x-1/2 -translate-y-1/2 gap-5 overflow-y-auto rounded-lg border border-[var(--border-strong)] bg-popover p-6 text-popover-foreground shadow-[var(--shadow-pop)] outline-none">
          <div className="grid gap-1.5">
            <Dialog.Title className="text-base font-semibold leading-6">
              Invite a member
            </Dialog.Title>
            <Dialog.Description className="text-sm leading-5 text-muted-foreground">
              Give someone access to {repoLabel}. Members can read all files,
              including private files, and take part in maintainer reviews.
            </Dialog.Description>
          </div>

          <form className="grid gap-5" onSubmit={(event) => void submit(event)}>
            <div className="grid gap-1.5">
              <label className="text-sm font-medium" htmlFor="invite-member-email">
                Email address
              </label>
              <Input
                autoComplete="off"
                id="invite-member-email"
                autoFocus
                disabled={pending}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="teammate@example.com"
                required
                type="email"
                value={email}
              />
            </div>

            <PermissionEditor
              disabled={pending}
              onChange={setPermissions}
              permissions={permissions}
            />

            <p className="text-sm leading-5 text-muted-foreground">
              We'll email an invitation that expires in 7 days. Access starts
              only after they accept.
            </p>

            {error && <p className="text-sm text-destructive" role="alert">{error}</p>}

            <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <Button
                disabled={pending}
                onClick={() => changeOpen(false)}
                size="sm"
                type="button"
                variant="secondary"
              >
                Cancel
              </Button>
              <Button disabled={pending || !email.trim()} size="sm" type="submit">
                {pending ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <Send className="size-3.5" />
                )}
                <span>Send invitation</span>
              </Button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
