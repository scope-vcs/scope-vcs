import type {
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowRunDetailResponse,
} from '@/api/types.generated'
import { useEffect, useRef } from 'react'
import { createCachedResource } from '../../lib/cached-resource'

const RECHECK_INTERVAL_MS = 15_000

export const githubWorkflowRunDetailResource = createCachedResource<GitHubWorkflowRunDetailResponse>({
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function githubWorkflowRunDetailIdentity(scope: string, runId: string) {
  return `${scope}\0${runId}`
}

export function invalidateGitHubWorkflowRunDetails(scope: string, runId?: number) {
  if (runId === undefined) {
    githubWorkflowRunDetailResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else {
    githubWorkflowRunDetailResource.invalidate(githubWorkflowRunDetailIdentity(scope, String(runId)))
  }
}

export const githubWorkflowJobLogResource = createCachedResource<GitHubWorkflowJobLogResponse>({
  maxEntries: 24,
  maxWeight: 16 * 1024 * 1024,
  weightOf: (value) => value.state === 'kept' ? value.text.length * 2 : 0,
})

export function githubWorkflowJobLogIdentity(scope: string, runId: string, jobKey: string) {
  return `${scope}\0${runId}\0${jobKey}`
}

export function useGitHubWorkflowRunRecheck(
  identity: string | null,
  mutable: boolean,
  pageValue: GitHubWorkflowRunDetailResponse | null,
) {
  const pageValueRef = useRef(pageValue)
  useEffect(() => {
    if (!identity) return
    const recheck = () => {
      if (!githubWorkflowRunDetailResource.getSnapshot(identity).pending) {
        githubWorkflowRunDetailResource.invalidate(identity)
      }
    }
    if (githubWorkflowRunDetailResource.peek(identity) !== pageValueRef.current) recheck()
    const onFocus = () => {
      if (document.visibilityState === 'visible') recheck()
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('online', recheck)
    document.addEventListener('visibilitychange', onFocus)
    const interval = mutable ? window.setInterval(recheck, RECHECK_INTERVAL_MS) : undefined
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('online', recheck)
      document.removeEventListener('visibilitychange', onFocus)
      window.clearInterval(interval)
    }
  }, [identity, mutable])
}
