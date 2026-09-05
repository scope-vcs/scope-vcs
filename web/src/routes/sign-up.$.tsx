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
  SignUp,
} from '@clerk/tanstack-react-start'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sign-up/$')({
  component: Page,
  errorComponent: AuthRouteError,
})

function Page() {
  return (
    <AuthLayout>
      <AuthSurface
        description="create an account for permissioned repository collaboration"
        title="Create your Scope account"
      >
        {forceSignedOut ? (
          <AuthFailureState title="sign up unavailable" />
        ) : (
          <>
            <ClerkLoading>
              <AuthLoadingState label="Loading sign up…" />
            </ClerkLoading>
            <ClerkFailed>
              <AuthFailureState title="Sign up unavailable" />
            </ClerkFailed>
            <ClerkLoaded>
              <div className="scope-content-enter">
                <SignUp />
              </div>
            </ClerkLoaded>
          </>
        )}
      </AuthSurface>
    </AuthLayout>
  )
}
