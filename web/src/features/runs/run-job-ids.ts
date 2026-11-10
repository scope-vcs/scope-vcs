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
  for (const key of keys) {
    if (runJobPanelId(key) !== parsed[1]) continue
    if (parsed[3] === undefined) return { key, attempt: null, step: null, line: null }
    let attempt: string | null = null
    try {
      attempt = parsed[2] === undefined ? null : decodeURIComponent(parsed[2])
    } catch {
      return null
    }
    return { key, attempt, step: Number(parsed[3]), line: parsed[4]! }
  }
  return null
}
