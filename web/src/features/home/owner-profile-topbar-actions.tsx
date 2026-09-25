import { Button } from '@/components/ui/button'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { KeyRound } from 'lucide-react'

/** Account controls in the profile topbar, shared with its pending state. */
export function OwnerProfileTopbarActions({ handle, signedIn }: { handle: string; signedIn: boolean }) {
  if (!signedIn) {
    return (
      <Button asChild size="sm" variant="secondary">
        <Link params={{ _splat: '' }} search={{ redirect_url: `/${handle}` }} to="/sign-in/$">
          Sign in
        </Link>
      </Button>
    )
  }
  return (
    <>
      <Button aria-label="CLI sessions" asChild size="icon-sm" title="CLI sessions" type="button" variant="ghost">
        <Link to="/account">
          <KeyRound />
        </Link>
      </Button>
      <UserButton />
    </>
  )
}
