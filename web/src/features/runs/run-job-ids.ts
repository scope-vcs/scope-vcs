export type RunJobLink = { key: string; step: number | null; line: string | null }

const RUN_HASH = /^#(run-job-[a-zA-Z0-9_-]+)(?::(\d+):L(\d+(?:\.\d+)?))?$/

export function runJobPanelId(jobKey: string) {
  return `run-job-${jobKey.replace(/[^a-zA-Z0-9_-]/g, '-')}`
}

export function runLogLineId(jobKey: string, step: number, line: string) {
  return `${runJobPanelId(jobKey)}:${step}:L${line}`
}

export function runJobLinkForHash(keys: Iterable<string>, hash: string): RunJobLink | null {
  const parsed = RUN_HASH.exec(hash)
  if (!parsed) return null
  const [, panelId, step, line] = parsed
  for (const key of keys) {
    if (runJobPanelId(key) !== panelId) continue
    if (step === undefined || line === undefined) return { key, step: null, line: null }
    return { key, step: Number(step), line }
  }
  return null
}
