import { definePlugin } from 'nitro'
import { secureResponse } from './security-headers'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  nitroApp.fetch = async (request) =>
    secureResponse(await fetch(request), process.env.NODE_ENV === 'production')
})
