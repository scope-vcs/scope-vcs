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
import { AuthLoadingState } from './auth-loading-state'

const authFormAppearance = {
  options: { elevation: 'flush' },
  elements: {
    rootBox: 'w-full [&_.cl-signIn-start_.cl-header]:hidden [&_.cl-signUp-start_.cl-header]:hidden',
    cardBox: 'w-full',
    socialButtonsBlockButton: 'h-11',
    lastAuthenticationStrategyBadge: 'end-2.5 [transform:translateY(calc(-50%_-_2px))]',
    formFieldInput: 'h-11 max-h-none',
    formButtonPrimary: 'h-11',
  },
} as const

const AUTH_FORMS = {
  'sign-in': { loading: 'Loading sign in…', Form: SignIn },
  'sign-up': { loading: 'Loading sign up…', Form: SignUp },
} as const

export function ClerkAuthRoute({ action }: { action: AuthAction }) {
  const { loading, Form } = AUTH_FORMS[action]
  return (
    <AuthLayout action={action}>
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
              <Form appearance={authFormAppearance} />
            </div>
          </ClerkLoaded>
        </>
      )}
    </AuthLayout>
  )
}
