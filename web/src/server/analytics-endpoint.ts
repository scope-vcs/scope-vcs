import { definePlugin } from 'nitro'
import { analyticsEndpointResponse } from './analytics-endpoint-handler'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  nitroApp.fetch = async (request) =>
    (await analyticsEndpointResponse(request)) ?? fetch(request)
})
