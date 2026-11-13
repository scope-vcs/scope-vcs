import { forceSignedOut } from '@/auth-mode'
import { fetchServerFunction } from '@/lib/stale-build'
import { publicRequestOrigin } from '@/server/public-origin'
import { serverFunctionFailureResponse } from '@/server/server-function-failure'
import { signedOutAuthMiddleware } from '@/server/signed-out-auth-middleware'
import { clerkMiddleware } from '@clerk/tanstack-react-start/server'
import { createCsrfMiddleware, createMiddleware, createStart, getGlobalStartContext } from '@tanstack/react-start'

const serverFunctionCsrfMiddleware = createCsrfMiddleware({
  filter: (ctx) => ctx.handlerType === 'serverFn',
  origin: (origin, ctx) =>
    origin === publicRequestOrigin(ctx.request.url, process.env.RAILWAY_ENVIRONMENT_ID),
})

const serverFunctionRequestFailureMiddleware = createMiddleware().server(async ({ handlerType, next }) => {
  if (handlerType !== 'serverFn') return next({ context: { serverFunctionRequest: false } })
  try {
    return await next({ context: { serverFunctionRequest: true } })
  } catch (error) {
    const response = serverFunctionFailureResponse(error)
    if (response) return response
    throw error
  }
})

const serverFunctionFailureMiddleware = createMiddleware({ type: 'function' }).server(async ({ next }) => {
  try {
    return await next()
  } catch (error) {
    if (!isServerFunctionRequest()) throw error
    throw serverFunctionFailureResponse(error) ?? error
  }
})

function isServerFunctionRequest() {
  const context: unknown = getGlobalStartContext()
  return typeof context === 'object' && context !== null
    && 'serverFunctionRequest' in context && context.serverFunctionRequest === true
}

export const startInstance = createStart(() => {
  return {
    functionMiddleware: [serverFunctionFailureMiddleware],
    requestMiddleware: [
      serverFunctionRequestFailureMiddleware,
      serverFunctionCsrfMiddleware,
      forceSignedOut ? signedOutAuthMiddleware : clerkMiddleware(),
    ],
    serverFns: { fetch: fetchServerFunction },
  }
})
