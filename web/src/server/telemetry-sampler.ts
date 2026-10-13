import { SpanKind } from '@opentelemetry/api'
import {
  ParentBasedSampler,
  SamplingDecision,
  type Sampler,
  type SamplingResult,
} from '@opentelemetry/sdk-trace-node'

class ServerRequestSampler implements Sampler {
  shouldSample(...[, , , kind]: Parameters<Sampler['shouldSample']>): SamplingResult {
    return {
      decision: kind === SpanKind.SERVER
        ? SamplingDecision.RECORD_AND_SAMPLED
        : SamplingDecision.NOT_RECORD,
    }
  }

  toString() {
    return 'ServerRequestSampler'
  }
}

export function requestTraceSampler(): Sampler {
  return new ParentBasedSampler({ root: new ServerRequestSampler() })
}
