const SHUTDOWN = Symbol.for('scope.web.telemetry-shutdown')

type TelemetryShutdownRegistry = { [SHUTDOWN]?: () => Promise<void> }

export function registerTelemetryShutdown(shutdown: () => Promise<void>) {
  ;(globalThis as TelemetryShutdownRegistry)[SHUTDOWN] = shutdown
}

export async function shutdownTelemetry() {
  await (globalThis as TelemetryShutdownRegistry)[SHUTDOWN]?.()
}
