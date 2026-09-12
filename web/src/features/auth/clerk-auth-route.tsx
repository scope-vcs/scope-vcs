import { forceSignedOut } from '@/auth-mode'
import {
  ClerkFailed,
  ClerkLoaded,
  ClerkLoading,
  SignIn,
  SignUp,
} from '@clerk/tanstack-react-start'
import { AuthFailureState, type AuthAction } from './auth-failure-state'
import { AuthLayout } from './auth-layout'
import { AuthLoadingState, AuthSurface } from './auth-loading-state'

const AUTH_SURFACES = {
  'sign-in': {
    description: 'continue to repositories, requests, and your CLI sessions',
    loading: 'Loading sign in…',
    title: 'Sign in to Scope',
    Form: SignIn,
  },
  'sign-up': {
    description: 'create an account for permissioned repository collaboration',
    loading: 'Loading sign up…',
    title: 'Create your Scope account',
    Form: SignUp,
  },
} as const

export function ClerkAuthRoute({ action }: { action: AuthAction }) {
  const { description, loading, title, Form } = AUTH_SURFACES[action]
  return (
    <AuthLayout>
      <AuthSurface description={description} title={title}>
        {forceSignedOut ? (
          <AuthFailureState action={action} />
        ) : (
          <>
            <ClerkLoading>
              <AuthLoadingState label={loading} />
            </ClerkLoading>
            <ClerkFailed>
              <AuthFailureState action={action} />
            </ClerkFailed>
            <ClerkLoaded>
              <div className="scope-content-enter">
                <Form />
              </div>
            </ClerkLoaded>
          </>
        )}
      </AuthSurface>
    </AuthLayout>
  )
}
