export type RunJobLink = { key: string; step: number | null; line: number | null }

const RUN_HASH = /^#(run-job-[a-zA-Z0-9_-]+)(?::(\d+):L(\d+))?$/

export function runJobPanelId(jobKey: string) {
  return `run-job-${jobKey.replace(/[^a-zA-Z0-9_-]/g, '-')}`
}

export function runLogLineId(jobKey: string, step: number, line: number) {
  return `${runJobPanelId(jobKey)}:${step}:L${line}`
}

export function runJobLinkForHash(keys: Iterable<string>, hash: string): RunJobLink | null {
  const parsed = RUN_HASH.exec(hash)
  if (!parsed) return null
  for (const key of keys) {
    if (runJobPanelId(key) !== parsed[1]) continue
    return parsed[2] === undefined
      ? { key, step: null, line: null }
      : { key, step: Number(parsed[2]), line: Number(parsed[3]) }
  }
  return null
}
