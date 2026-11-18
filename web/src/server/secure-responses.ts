import { randomBytes } from 'node:crypto'
import { definePlugin } from 'nitro'
import { forceSignedOut } from '../auth-mode'
import { clerkFrontendOrigin, CSP_NONCE_HEADER, secureResponse } from './security-headers'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  const clerkOrigin = forceSignedOut
    ? null
    : clerkFrontendOrigin(process.env.VITE_CLERK_PUBLISHABLE_KEY ?? process.env.CLERK_PUBLISHABLE_KEY)
  nitroApp.fetch = async (request) => {
    const nonce = randomBytes(18).toString('base64')
    request.headers.set(CSP_NONCE_HEADER, nonce)
    return secureResponse(await fetch(request), {
      clerkOrigin,
      nonce,
      production: process.env.NODE_ENV === 'production',
    })
  }
})
