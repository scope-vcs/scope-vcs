const GRANT_RENEWAL_MARGIN_MS = 30_000
const AUTOMATIC_RECOVERY_INTERVAL_MS = 60_000

type MediaGrant = { expires_at_unix: number }

export type MediaRecovery = {
  failed: boolean
  generation: number
  lastAttemptAt: number | null
  renewingFrom: MediaGrant | null
}

export const initialMediaRecovery: MediaRecovery = {
  failed: false,
  generation: 0,
  lastAttemptAt: null,
  renewingFrom: null,
}

export function mediaGrantExpiring(grant: MediaGrant, now: number) {
  return grant.expires_at_unix * 1_000 - now <= GRANT_RENEWAL_MARGIN_MS
}

export function settledMediaRecovery(recovery: MediaRecovery, grant: MediaGrant | null): MediaRecovery {
  if (recovery.renewingFrom === null || grant === null || grant === recovery.renewingFrom) return recovery
  return { ...recovery, failed: false, generation: recovery.generation + 1, renewingFrom: null }
}

export function recoverFromMediaError(
  current: MediaRecovery,
  { generation, grant, now }: { generation: number; grant: MediaGrant; now: number },
) {
  const recovery = settledMediaRecovery(current, grant)
  if (recovery.failed || recovery.renewingFrom !== null || generation !== recovery.generation) {
    return { recovery, renew: false }
  }
  if (recovery.lastAttemptAt !== null && now - recovery.lastAttemptAt < AUTOMATIC_RECOVERY_INTERVAL_MS) {
    return { recovery: { ...recovery, failed: true }, renew: false }
  }
  return reloadMedia(recovery, grant, now, false)
}

export function retryMedia(current: MediaRecovery, grant: MediaGrant | null, now: number) {
  const recovery = settledMediaRecovery(current, grant)
  if (grant === null || recovery.renewingFrom !== null) return { recovery, renew: true }
  return reloadMedia(recovery, grant, now, true)
}

function reloadMedia(recovery: MediaRecovery, grant: MediaGrant, now: number, keepHidden: boolean) {
  if (mediaGrantExpiring(grant, now)) {
    return { recovery: { ...recovery, failed: keepHidden, lastAttemptAt: now, renewingFrom: grant }, renew: true }
  }
  return {
    recovery: { ...recovery, failed: false, generation: recovery.generation + 1, lastAttemptAt: now },
    renew: false,
  }
}
