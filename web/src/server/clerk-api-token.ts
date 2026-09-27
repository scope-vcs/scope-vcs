import { auth } from '@clerk/tanstack-react-start/server'
import { createClerkApiTokenCache } from './clerk-api-token-cache'

const clerkApiTokens = createClerkApiTokenCache()

export async function readClerkApiToken(template: string) {
  const session = await auth()
  if (!session.isAuthenticated) {
    return null
  }
  return clerkApiTokens.read(
    { sessionId: session.sessionId, template, userId: session.userId },
    () => session.getToken({ template }),
  )
}
