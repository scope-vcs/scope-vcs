export type RunJobLink = { key: string; attempt: string | null; step: number | null; line: string | null }

const RUN_HASH = /^#(run-job-[a-zA-Z0-9_-]+)(?::(?:([^:]+):)?(\d+):L(\d+(?:\.\d+)?))?$/

export function runJobPanelId(jobKey: string) {
  return `run-job-${jobKey.replace(/[^a-zA-Z0-9_-]/g, '-')}`
}

export function runLogLineId(jobKey: string, attempt: string | null, step: number, line: string) {
  const attemptPart = attempt === null ? '' : `${encodeURIComponent(attempt)}:`
  return `${runJobPanelId(jobKey)}:${attemptPart}${step}:L${line}`
}

export function runJobLinkForHash(keys: Iterable<string>, hash: string): RunJobLink | null {
  const parsed = RUN_HASH.exec(hash)
  if (!parsed) return null
  const [, panelId, encodedAttempt, step, line] = parsed
  for (const key of keys) {
    if (runJobPanelId(key) !== panelId) continue
    if (step === undefined || line === undefined) return { key, attempt: null, step: null, line: null }
    let attempt: string | null = null
    try {
      attempt = encodedAttempt === undefined ? null : decodeURIComponent(encodedAttempt)
    } catch {
      return null
    }
    return { key, attempt, step: Number(step), line }
  }
  return null
}
