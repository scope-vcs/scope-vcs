import { AuthRouteError } from '@/features/auth/auth-failure-state'
import { ClerkAuthRoute } from '@/features/auth/clerk-auth-route'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-in/$')({
  component: () => <ClerkAuthRoute action="sign-in" />,
  errorComponent: () => <AuthRouteError action="sign-in" />,
})
