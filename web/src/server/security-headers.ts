export const WEB_CONTENT_SECURITY_POLICY =
  "frame-ancestors 'none'; object-src 'none'; base-uri 'self'"

const HSTS_MAX_AGE_SECONDS = 365 * 24 * 60 * 60

export function secureResponse(response: Response, production: boolean) {
  const headers = new Headers(response.headers)
  headers.append('content-security-policy', WEB_CONTENT_SECURITY_POLICY)
  headers.set('x-content-type-options', 'nosniff')
  headers.set('x-frame-options', 'DENY')
  headers.set('referrer-policy', 'strict-origin-when-cross-origin')
  if (production) headers.set('strict-transport-security', `max-age=${HSTS_MAX_AGE_SECONDS}`)

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  })
}
