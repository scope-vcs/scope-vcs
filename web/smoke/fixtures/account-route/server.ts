export function createServerFn({ method }: { method: string }) {
  let revocation = false
  return {
    handler: () => async ({ data }: { data?: { sessionId: string } } = {}) => {
      if (method === 'GET') return window.accountRouteLoaded
      if (revocation) {
        if (window.failRevoke) throw new Error('Revocation denied')
        window.removeAccountSession(data!.sessionId)
      }
    },
    validator() { revocation = true; return this },
  }
}
