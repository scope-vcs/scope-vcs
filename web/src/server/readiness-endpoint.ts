import { definePlugin } from 'nitro'
import { readinessResponse } from './readiness'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  nitroApp.fetch = async (request) => {
    const url = new URL(request.url)
    if (request.method === 'GET' && url.pathname === '/readyz') {
      return readinessResponse()
    }
    return fetch(request)
  }
})
