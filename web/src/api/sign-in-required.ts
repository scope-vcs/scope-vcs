export const SIGN_IN_REQUIRED_HEADER = 'x-scope-sign-in-required'

export class SignInRequiredError extends Error {
  constructor() {
    super('Sign in required.')
    this.name = 'SignInRequiredError'
  }
}
