import { writeFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

export const volumeInstanceId = 'd849de88-290f-4f68-a6bb-33a8bac7c0fd'
const maximumAgeMs = 30 * 60 * 60 * 1000
class BackupCheckError extends Error {}

export function assessBackups(data, now = Date.now()) {
  if (!Array.isArray(data?.volumeInstanceBackupList) || !Array.isArray(data?.volumeInstanceBackupScheduleList)) {
    throw new BackupCheckError('Railway returned an invalid backup response')
  }
  const failures = []
  const kinds = new Set(data.volumeInstanceBackupScheduleList.map(schedule => schedule.kind))
  for (const kind of ['DAILY', 'WEEKLY']) {
    if (!kinds.has(kind)) failures.push(`${kind} production backup schedule is missing`)
  }
  const snapshots = data.volumeInstanceBackupList
    .map(backup => ({ createdAt: Date.parse(backup.createdAt), expiresAt: backup.expiresAt ? Date.parse(backup.expiresAt) : Infinity }))
    .filter(backup => Number.isFinite(backup.createdAt) && backup.createdAt <= now && backup.expiresAt > now)
  const latest = snapshots.length ? Math.max(...snapshots.map(backup => backup.createdAt)) : null
  if (latest === null) failures.push('No unexpired completed production backup is available')
  else if (now - latest > maximumAgeMs) failures.push(`Latest completed production backup is ${Math.floor((now - latest) / 3600000)} hours old; limit is 30 hours`)
  return { healthy: failures.length === 0, failures, latestBackupAt: latest === null ? null : new Date(latest).toISOString() }
}

export async function fetchBackupHealth(token, fetcher = fetch) {
  if (!token) throw new BackupCheckError('Production Railway project token is missing')
  const response = await fetcher('https://backboard.railway.com/graphql/v2', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'project-access-token': token },
    body: JSON.stringify({
      query: `query ProductionBackupHealth($volumeInstanceId: String!) {
        volumeInstanceBackupList(volumeInstanceId: $volumeInstanceId) { id createdAt expiresAt }
        volumeInstanceBackupScheduleList(volumeInstanceId: $volumeInstanceId) { id kind }
      }`,
      variables: { volumeInstanceId },
    }),
    signal: AbortSignal.timeout(30000),
  })
  // Do not print API errors or response bodies, which can contain sensitive metadata.
  if (!response.ok) throw new BackupCheckError(`Railway backup query failed with HTTP ${response.status}`)
  const body = await response.json()
  if (body.errors?.length) throw new BackupCheckError('Railway rejected the backup query; inspect project-token access and schema')
  return assessBackups(body.data)
}

export async function main() {
  let result
  try {
    result = await fetchBackupHealth(process.env.RAILWAY_TOKEN)
  } catch (error) {
    result = { healthy: false, failures: [error instanceof BackupCheckError ? error.message : 'Backup query failed; inspect connectivity and Railway API availability'], latestBackupAt: null }
  }
  const metrics = [{ MetricName: 'Healthy', Dimensions: [{ Name: 'VolumeInstanceId', Value: volumeInstanceId }], Value: result.healthy ? 1 : 0, Unit: 'Count' }]
  await writeFile(process.env.BACKUP_METRIC_FILE ?? 'backup-health-metric.json', JSON.stringify(metrics))
  console.log(JSON.stringify(result))
  if (!result.healthy) process.exitCode = 1
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main()
