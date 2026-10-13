import { SpanStatusCode } from '@opentelemetry/api'
import type { ExportResult } from '@opentelemetry/core'
import type { ReadableSpan, SpanExporter } from '@opentelemetry/sdk-trace-node'

const EXPORTED_ATTRIBUTES = new Set([
  'http.request.method',
  'http.response.status_code',
  'http.route',
  'network.protocol.version',
  'server.address',
  'server.port',
  'url.scheme',
])

export class AllowedAttributesExporter implements SpanExporter {
  constructor(private readonly exporter: SpanExporter) {}

  export(spans: ReadableSpan[], resultCallback: (result: ExportResult) => void) {
    this.exporter.export(spans.map(keepAllowedAttributes), resultCallback)
  }

  shutdown() {
    return this.exporter.shutdown()
  }

  forceFlush() {
    return this.exporter.forceFlush?.() ?? Promise.resolve()
  }
}

function keepAllowedAttributes(span: ReadableSpan): ReadableSpan {
  return {
    attributes: Object.fromEntries(
      Object.entries(span.attributes).filter(([name]) => EXPORTED_ATTRIBUTES.has(name)),
    ),
    droppedAttributesCount: span.droppedAttributesCount,
    droppedEventsCount: 0,
    droppedLinksCount: span.droppedLinksCount,
    duration: span.duration,
    ended: span.ended,
    endTime: span.endTime,
    events: [],
    instrumentationScope: span.instrumentationScope,
    kind: span.kind,
    links: span.links,
    name: span.name,
    parentSpanContext: span.parentSpanContext,
    resource: span.resource,
    spanContext: () => span.spanContext(),
    startTime: span.startTime,
    status: span.status.code === SpanStatusCode.ERROR
      ? { code: SpanStatusCode.ERROR, message: '' }
      : span.status,
  }
}
