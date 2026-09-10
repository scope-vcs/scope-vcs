import { loadRepoDependenciesForRequest } from '@/api/repos'
import { parseRepoParams } from '@/api/repo-params'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

export const loadRepositoryDependencies = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoDependenciesForRequest(data, getRequest().signal))
