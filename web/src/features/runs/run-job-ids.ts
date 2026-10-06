export function runJobPanelId(jobKey: string) {
  return `run-job-${jobKey.replace(/[^a-zA-Z0-9_-]/g, '-')}`
}

export function jobKeyForHash(keys: Iterable<string>, hash: string): string | null {
  if (!hash.startsWith('#run-job-')) return null
  for (const key of keys) {
    if (`#${runJobPanelId(key)}` === hash) return key
  }
  return null
}
