import { displayRouteFilePath } from '../../lib/route-file'

type RepositoryPath = { path: string }

export const REPOSITORY_FILE_MATCH_LIMIT = 100

export function matchingRepositoryFiles<T extends RepositoryPath>(files: readonly T[], query: string) {
  const needle = query.trim().toLowerCase()
  const matches = files.filter((file) => displayRouteFilePath(file.path).toLowerCase().includes(needle))
  return { shown: matches.slice(0, REPOSITORY_FILE_MATCH_LIMIT), total: matches.length }
}

