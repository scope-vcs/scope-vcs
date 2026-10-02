import { forceSignedOut } from '@/auth-mode'
import { fetchServerFunction } from '@/lib/stale-build'
import { publicRequestOrigin } from '@/server/public-origin'
import { signedOutAuthMiddleware } from '@/server/signed-out-auth-middleware'
import { staleServerFunctionResponse } from '@/server/stale-server-function'
import { clerkMiddleware } from '@clerk/tanstack-react-start/server'
import { createCsrfMiddleware, createMiddleware, createStart } from '@tanstack/react-start'

const serverFunctionCsrfMiddleware = createCsrfMiddleware({
  filter: (ctx) => ctx.handlerType === 'serverFn',
  origin: (origin, ctx) =>
    origin === publicRequestOrigin(ctx.request.url, process.env.RAILWAY_ENVIRONMENT_ID),
})

const staleServerFunctionMiddleware = createMiddleware().server(async ({ handlerType, next }) => {
  if (handlerType !== 'serverFn') return next()
  try {
    return await next()
  } catch (error) {
    const response = staleServerFunctionResponse(error)
    if (response) return response
    throw error
  }
})

export const startInstance = createStart(() => {
  return {
    requestMiddleware: [
      staleServerFunctionMiddleware,
      serverFunctionCsrfMiddleware,
      forceSignedOut ? signedOutAuthMiddleware : clerkMiddleware(),
    ],
    serverFns: { fetch: fetchServerFunction },
  }
})
