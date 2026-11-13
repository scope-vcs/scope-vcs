export type RunTone = 'running' | 'success' | 'danger' | 'waiting' | 'inert'

type RunStatus = {
  animated: boolean
  label: string
  tone: RunTone
}

const TONES: Record<string, RunTone> = {
  failed: 'danger',
  pending: 'waiting',
  queued: 'waiting',
  running: 'running',
  skipped: 'inert',
  succeeded: 'success',
}

export function runStatus(state: string): RunStatus {
  const tone = TONES[state] ?? 'waiting'
  return { animated: tone === 'running', label: state, tone }
}

const DURATION_JOINERS: Record<RunTone, string> = {
  danger: 'after',
  inert: 'after',
  running: 'for',
  success: 'in',
  waiting: 'for',
}

export function runDurationLead(state: string, label = runStatus(state).label) {
  const { tone } = runStatus(state)
  return `${label.charAt(0).toUpperCase()}${label.slice(1)} ${DURATION_JOINERS[tone]}`
}
