import { SIGN_IN_REQUIRED_HEADER } from '../api/sign-in-required'

export const SESSION_READY_BOUND_MS = 10_000

export type SessionReadiness = {
  ready: () => boolean
  whenReady: (boundMs: number) => Promise<void>
}

export function withSessionRetry(send: typeof fetch, session: SessionReadiness): typeof fetch {
  return async (input, init) => {
    const readyAtSend = session.ready()
    const response = await send(input, init)
    if (readyAtSend || !response.headers.has(SIGN_IN_REQUIRED_HEADER)) return response
    await response.body?.cancel()
    await session.whenReady(SESSION_READY_BOUND_MS)
    return send(input, init)
  }
}
