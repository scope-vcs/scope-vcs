export function matchesCsrfOrigin(
  origin: string,
  requestUrl: string,
  railwayEnvironmentId: string | undefined,
) {
  const url = new URL(requestUrl)
  // Railway terminates HTTPS at the edge, so Nitro sees an HTTP request URL.
  // Use the deployment environment rather than trusting forwarded headers.
  if (railwayEnvironmentId) url.protocol = 'https:'
  return origin === url.origin
}
