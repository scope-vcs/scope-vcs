import { Button } from '@/components/ui/button'
import { UserButton, useAuth } from '@clerk/tanstack-react-start'
import { Link, useLocation } from '@tanstack/react-router'
import { Settings } from 'lucide-react'

export function AccountControl() {
  const { isLoaded, isSignedIn } = useAuth()
  const href = useLocation({ select: (location) => location.href })

  if (!isLoaded) return null
  if (isSignedIn) {
    return (
      <UserButton>
        <UserButton.MenuItems>
          <UserButton.Link
            href="/settings"
            label="Settings"
            labelIcon={<Settings className="size-4" />}
          />
        </UserButton.MenuItems>
      </UserButton>
    )
  }
  return (
    <Button asChild size="sm" variant="secondary">
      <Link params={{ _splat: '' }} search={{ redirect_url: href }} to="/sign-in/$">
        Sign in
      </Link>
    </Button>
  )
}
