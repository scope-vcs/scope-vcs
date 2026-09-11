import { AuthRouteError } from '@/features/auth/auth-failure-state'
import { ClerkAuthRoute } from '@/features/auth/clerk-auth-route'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-up/$')({
  component: () => <ClerkAuthRoute action="sign-up" />,
  errorComponent: () => <AuthRouteError action="sign-up" />,
})
