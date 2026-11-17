export const CSP_NONCE_HEADER = 'x-scope-csp-nonce'

const HSTS_MAX_AGE_SECONDS = 365 * 24 * 60 * 60
const CLERK_CHALLENGE_ORIGIN = 'https://challenges.cloudflare.com'
const CLERK_PROTECTION_HOSTS = 'https://*.protect.clerk.com'
const LOCAL_DEVELOPMENT_SOURCES = 'http://localhost:* http://127.0.0.1:* ws://localhost:* ws://127.0.0.1:*'

type ResponseSecurity = {
  clerkOrigin: string | null
  nonce: string
  production: boolean
}

export function clerkFrontendOrigin(publishableKey: string | undefined) {
  const encoded = publishableKey?.match(/^pk_(?:test|live)_(.+)$/)?.[1]
  if (!encoded) return null
  const host = Buffer.from(encoded, 'base64').toString('utf8').replace(/\$$/, '')
  return /^[a-z0-9.-]+$/i.test(host) ? `https://${host}` : null
}

export function contentSecurityPolicy({ clerkOrigin, nonce, production }: ResponseSecurity) {
  const local = production ? [] : [LOCAL_DEVELOPMENT_SOURCES]
  const scripts = [
    "'self'",
    `'nonce-${nonce}'`,
    "'wasm-unsafe-eval'",
    ...clerkOrigin ? [clerkOrigin, CLERK_CHALLENGE_ORIGIN, CLERK_PROTECTION_HOSTS] : [],
  ]
  const clerkFrames = clerkOrigin ? [CLERK_CHALLENGE_ORIGIN, CLERK_PROTECTION_HOSTS] : []
  const clerkConnections = clerkOrigin ? [`${CLERK_PROTECTION_HOSTS}:*`] : []
  const directives = [
    ["default-src", "'self'"],
    ["script-src", ...scripts],
    ["style-src", "'self'", "'unsafe-inline'"],
    ["img-src", "'self'", 'data:', 'blob:', 'https:', ...local],
    ["media-src", "'self'", 'data:', 'blob:', 'https:', ...local],
    ["font-src", "'self'", 'data:'],
    ["connect-src", "'self'", 'https:', ...clerkConnections, ...local],
    ["worker-src", "'self'", 'blob:'],
    ["frame-src", "'self'", ...clerkFrames],
    ["form-action", "'self'"],
    ["frame-ancestors", "'none'"],
    ["object-src", "'none'"],
    ["base-uri", "'self'"],
  ]
  return directives.map((directive) => directive.join(' ')).join('; ')
}

export function secureResponse(response: Response, security: ResponseSecurity) {
  const headers = new Headers(response.headers)
  headers.append('content-security-policy', contentSecurityPolicy(security))
  headers.set('x-content-type-options', 'nosniff')
  headers.set('x-frame-options', 'DENY')
  headers.set('referrer-policy', 'strict-origin-when-cross-origin')
  if (security.production) headers.set('strict-transport-security', `max-age=${HSTS_MAX_AGE_SECONDS}`)

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  })
}
