import { logs, SeverityNumber, type AnyValueMap } from '@opentelemetry/api-logs'

const SEVERITIES = {
  error: { number: SeverityNumber.ERROR, text: 'ERROR' },
  info: { number: SeverityNumber.INFO, text: 'INFO' },
  warn: { number: SeverityNumber.WARN, text: 'WARN' },
} as const

const logger = logs.getLogger('scope-web')

export function serverLog(
  level: keyof typeof SEVERITIES,
  message: string,
  attributes: AnyValueMap = {},
) {
  console[level](message, attributes)
  logger.emit({
    attributes,
    body: message,
    severityNumber: SEVERITIES[level].number,
    severityText: SEVERITIES[level].text,
  })
}
