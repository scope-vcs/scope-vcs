import type { DeleteAccountResult } from '@/api/account'
import { TypedConfirmationDialog } from '@/components/typed-confirmation-dialog'
import { storeHomeFlash } from '@/lib/home-flash'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { useAuth, useClerk } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { useState, type ReactNode } from 'react'
import { AccountDangerZoneSection } from './account-sections'
import { resetAccountSessionResource } from './account-session-resource'
import { useAccountSession } from './use-account-session'

export function DeleteAccountSection({
  deleteAccount,
}: {
  deleteAccount: () => Promise<DeleteAccountResult>
}) {
  const clerk = useClerk()
  const { userId } = useAuth()
  const handle = useAccountSession(userId ?? null).value?.account?.user?.handle ?? null
  const [open, setOpen] = useState(false)
  const [error, setError] = useState<ReactNode>(null)

  async function confirm() {
    setError(null)
    let result: DeleteAccountResult
    try {
      result = await deleteAccount()
    } catch (error) {
      setError(resourceErrorMessage(error, 'Account deletion failed.'))
      return
    }
    if (result.status === 'blocked') {
      setError(<SharedRepositories repositories={result.repositories} />)
      return
    }
    resetAccountSessionResource()
    storeHomeFlash('Your Scope account was deleted.')
    await clerk.signOut({ redirectUrl: '/' })
  }

  return (
    <>
      <AccountDangerZoneSection onDelete={handle === null ? undefined : () => setOpen(true)} />
      {open && handle !== null && (
        <TypedConfirmationDialog
          confirmLabel="Delete account"
          confirmation={handle}
          error={error}
          onCancel={() => {
            setOpen(false)
            setError(null)
          }}
          onConfirm={confirm}
          purpose="permanently delete your account"
          subject={`@${handle}`}
          title="Delete account"
          warning="This permanently deletes your Scope account and every repository you own. It cannot be undone."
        />
      )}
    </>
  )
}

function SharedRepositories({ repositories }: { repositories: string[] }) {
  return (
    <div className="space-y-1.5">
      <p>Other members use these repositories. Delete them first.</p>
      <ul className="space-y-1 text-foreground">
        {repositories.map((id) => {
          const [owner = '', repo = ''] = id.split('/')
          return (
            <li key={id}>
              <Link
                className="font-mono text-xs underline-offset-4 hover:underline"
                params={{ owner, repo }}
                to="/$owner/$repo/settings"
              >
                {id}
              </Link>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
