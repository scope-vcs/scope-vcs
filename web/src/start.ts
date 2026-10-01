import { forceSignedOut } from '@/auth-mode'
import { publicRequestOrigin } from '@/server/public-origin'
import { signedOutAuthMiddleware } from '@/server/signed-out-auth-middleware'
import { clerkMiddleware } from '@clerk/tanstack-react-start/server'
import { createCsrfMiddleware, createStart } from '@tanstack/react-start'

const serverFunctionCsrfMiddleware = createCsrfMiddleware({
  filter: (ctx) => ctx.handlerType === 'serverFn',
  origin: (origin, ctx) =>
    origin === publicRequestOrigin(ctx.request.url, process.env.RAILWAY_ENVIRONMENT_ID),
})

export const startInstance = createStart(() => {
  return {
    requestMiddleware: [
      serverFunctionCsrfMiddleware,
      forceSignedOut ? signedOutAuthMiddleware : clerkMiddleware(),
    ],
  }
})
