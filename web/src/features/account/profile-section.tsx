import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useAuth, useClerk, useUser } from '@clerk/tanstack-react-start'
import { Check, LogOut } from 'lucide-react'
import { SettingsSection } from './settings-sections'
import { useAccountSession } from './use-account-session'

export function ProfileSection() {
  const clerk = useClerk()
  const { userId } = useAuth()
  const { user: clerkUser } = useUser()
  const session = useAccountSession(userId ?? null)
  const user = session.value?.account?.user ?? null

  return (
    <SettingsSection id="profile" title="Profile">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        {clerkUser ? (
          <img alt="" className="size-11 shrink-0 rounded-full bg-muted" src={clerkUser.imageUrl} />
        ) : (
          <BlockSkeleton className="size-11 shrink-0 rounded-full" />
        )}
        <div className="min-w-0">
          {user ? (
            <>
              <div className="truncate font-semibold leading-6">@{user.handle}</div>
              <div className="flex flex-wrap items-center gap-x-2 text-sm leading-5 text-muted-foreground">
                <span className="truncate">{user.email}</span>
                {user.email_verified ? (
                  <span className="inline-flex items-center gap-1 text-xs text-success">
                    <Check aria-hidden className="size-3" />
                    Verified
                  </span>
                ) : (
                  <span className="text-xs text-warning-strong">Not verified</span>
                )}
              </div>
            </>
          ) : session.error ? (
            <p className="text-sm text-muted-foreground">{session.error}</p>
          ) : (
            <>
              <TextSkeleton length="short" />
              <TextSkeleton className="mt-1" length="medium" size="meta" />
            </>
          )}
        </div>
        <div className="flex gap-2 sm:ml-auto">
          <Button onClick={() => clerk.openUserProfile()} size="sm" type="button" variant="secondary">
            Manage sign-in
          </Button>
          <Button
            onClick={() => void clerk.signOut({ redirectUrl: '/' })}
            size="sm"
            type="button"
            variant="secondary"
          >
            <LogOut />
            Sign out
          </Button>
        </div>
      </div>
    </SettingsSection>
  )
}
