// Keep resource directives out of this baseline until Clerk authentication,
// analytics, workers, and repository media have a verified nonce/source policy.
// These directives protect framing, plugins, and base URL injection without
// changing the application's permitted scripts or network destinations.
export const WEB_CONTENT_SECURITY_POLICY =
  "frame-ancestors 'none'; object-src 'none'; base-uri 'self'"

export function secureResponse(response: Response, production: boolean) {
  const headers = new Headers(response.headers)
  // Appending keeps any stricter route-specific policy in force as well.
  headers.append('content-security-policy', WEB_CONTENT_SECURITY_POLICY)
  headers.set('x-content-type-options', 'nosniff')
  headers.set('x-frame-options', 'DENY')
  headers.set('referrer-policy', 'strict-origin-when-cross-origin')
  // Railway terminates TLS before the request reaches Nitro. Do not depend on
  // forwarded request headers; production is served over HTTPS at the edge.
  // Subdomains and preload need a separate inventory before opting in.
  if (production) headers.set('strict-transport-security', 'max-age=31536000')

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  })
}
