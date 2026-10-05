export function publicRequestOrigin(
  requestUrl: string | URL,
  railwayEnvironmentId: string | undefined,
) {
  const url = new URL(requestUrl)
  if (railwayEnvironmentId) url.protocol = 'https:'
  return url.origin
}
