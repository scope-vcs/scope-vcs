import { createRequire } from 'node:module'
import { W3CTraceContextPropagator } from '@opentelemetry/core'
import { OTLPLogExporter } from '@opentelemetry/exporter-logs-otlp-http'
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http'
import { logs } from '@opentelemetry/api-logs'
import { registerInstrumentations } from '@opentelemetry/instrumentation'
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http'
import { UndiciInstrumentation } from '@opentelemetry/instrumentation-undici'
import { resourceFromAttributes } from '@opentelemetry/resources'
import { BatchLogRecordProcessor, LoggerProvider } from '@opentelemetry/sdk-logs'
import { BatchSpanProcessor, NodeTracerProvider } from '@opentelemetry/sdk-trace-node'
import { AllowedAttributesExporter } from './telemetry-allowed-attributes'
import { getTelemetryConfig, INJECTED_OTLP_VARIABLES_MESSAGE } from './telemetry-config'
import { requestTraceSampler } from './telemetry-sampler'
import { registerTelemetryShutdown } from './telemetry-shutdown'

const UNTRACED_PATHS = new Set(['/healthz', '/readyz'])

const config = getTelemetryConfig()

function patchHttpServerModules() {
  const requireBuiltin = createRequire(import.meta.url)
  requireBuiltin('http')
  requireBuiltin('https')
}

if (config.enabled) {
  const resource = resourceFromAttributes(config.resourceAttributes)
  const tracerProvider = new NodeTracerProvider({
    resource,
    sampler: requestTraceSampler(),
    spanProcessors: [
      new BatchSpanProcessor(new AllowedAttributesExporter(new OTLPTraceExporter({
        headers: config.headers,
        url: config.tracesUrl,
      }))),
    ],
  })
  tracerProvider.register({ propagator: new W3CTraceContextPropagator() })

  const loggerProvider = new LoggerProvider({
    processors: [
      new BatchLogRecordProcessor({
        exporter: new OTLPLogExporter({
          headers: config.headers,
          url: config.logsUrl,
        }),
      }),
    ],
    resource,
  })
  logs.setGlobalLoggerProvider(loggerProvider)

  registerInstrumentations({
    instrumentations: [
      new HttpInstrumentation({
        disableOutgoingRequestInstrumentation: true,
        ignoreIncomingRequestHook: (request) =>
          UNTRACED_PATHS.has(new URL(request.url ?? '/', 'http://localhost').pathname),
      }),
      new UndiciInstrumentation(),
    ],
  })

  patchHttpServerModules()

  registerTelemetryShutdown(async () => {
    await Promise.allSettled([tracerProvider.shutdown(), loggerProvider.shutdown()])
  })
} else if (config.reason === 'injected-otlp-variables') {
  console.error(INJECTED_OTLP_VARIABLES_MESSAGE)
}
