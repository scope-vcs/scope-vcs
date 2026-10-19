export type MediaRecovery = {
  failed: string | null
  renewedFrom: string | null
}

export const initialMediaRecovery: MediaRecovery = { failed: null, renewedFrom: null }

export function recoverFromMediaError(recovery: MediaRecovery, source: string) {
  if (source === recovery.renewedFrom || source === recovery.failed) {
    return { recovery, renew: false }
  }
  if (recovery.renewedFrom !== null) {
    return { recovery: { ...recovery, failed: source }, renew: false }
  }
  return { recovery: { failed: null, renewedFrom: source }, renew: true }
}

export function recoverFromMediaLoad(recovery: MediaRecovery, source: string) {
  return source === recovery.renewedFrom ? recovery : initialMediaRecovery
}

export function recoverByRetry(recovery: MediaRecovery): MediaRecovery {
  const source = recovery.failed ?? recovery.renewedFrom
  return { failed: source, renewedFrom: source }
}
