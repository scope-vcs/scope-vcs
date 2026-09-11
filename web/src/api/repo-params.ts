import type { RepoParams } from './types'

export function parseRepoParams(input: unknown): RepoParams {
  const data = input as Partial<RepoParams> | null
  return {
    owner: repoSegment(data?.owner),
    repo: repoSegment(data?.repo),
  }
}

function repoSegment(value: unknown) {
  const segment = typeof value === 'string' ? value.trim() : ''
  if (!segment) {
    throw new Error('Repository route is incomplete.')
  }
  if (segment.includes('/')) {
    throw new Error('Repository owner and name must be single path segments.')
  }
  return segment
}
