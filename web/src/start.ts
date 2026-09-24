import { forceSignedOut } from '@/auth-mode'
import { matchesCsrfOrigin } from '@/server/csrf-origin'
import { signedOutAuthMiddleware } from '@/server/signed-out-auth-middleware'
import { clerkMiddleware } from '@clerk/tanstack-react-start/server'
import { createCsrfMiddleware, createStart } from '@tanstack/react-start'

const serverFunctionCsrfMiddleware = createCsrfMiddleware({
  filter: (ctx) => ctx.handlerType === 'serverFn',
  origin: (origin, ctx) => matchesCsrfOrigin(origin, ctx.request.url, process.env.RAILWAY_ENVIRONMENT_ID),
})

export const startInstance = createStart(() => {
  return {
    requestMiddleware: [
      serverFunctionCsrfMiddleware,
      forceSignedOut ? signedOutAuthMiddleware : clerkMiddleware(),
    ],
  }
})
