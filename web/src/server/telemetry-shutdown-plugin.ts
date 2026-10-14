import { definePlugin } from 'nitro'
import { shutdownTelemetry } from './telemetry-shutdown'

export default definePlugin((nitroApp) => {
  nitroApp.hooks.hook('close', shutdownTelemetry)
})
