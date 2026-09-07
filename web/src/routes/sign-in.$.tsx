import { AuthLayout } from '@/features/auth/auth-layout'
import { forceSignedOut } from '@/auth-mode'
import { AuthFailureState, AuthRouteError } from '@/features/auth/auth-failure-state'
import {
  AuthLoadingState,
  AuthSurface,
} from '@/features/auth/auth-loading-state'
import {
  ClerkFailed,
  ClerkLoaded,
  ClerkLoading,
  SignIn,
} from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-in/$')({
  component: Page,
  errorComponent: RouteError,
})

function Page() {
  return (
    <AuthLayout>
      <AuthSurface
        description="continue to repositories, requests, and your CLI sessions"
        title="Sign in to Scope"
      >
        {forceSignedOut ? (
          <AuthFailureState action="sign-in" />
        ) : (
          <>
            <ClerkLoading>
              <AuthLoadingState label="Loading sign in…" />
            </ClerkLoading>
            <ClerkFailed>
              <AuthFailureState action="sign-in" />
            </ClerkFailed>
            <ClerkLoaded>
              <div className="scope-content-enter">
                <SignIn />
              </div>
            </ClerkLoaded>
          </>
        )}
      </AuthSurface>
    </AuthLayout>
  )
}

function RouteError() {
  return <AuthRouteError action="sign-in" />
}
