export function createServerFn({ method }: { method: string }) {
  let revocation = false
  return {
    handler: () => async ({ data }: { data?: { sessionId: string } } = {}) => {
      if (method === 'GET') {
        const response = window.accountRouteResponse
        if (window.delayAccountResponse) {
          window.delayAccountResponse = false
          return new Promise((resolve) => {
            window.deliverAccountResponse = () => resolve(response)
          })
        }
        return response
      }
      if (revocation) {
        if (window.failRevoke) throw new Error('Revocation denied')
        window.removeAccountSession(data!.sessionId)
      }
    },
    validator() { revocation = true; return this },
  }
}
