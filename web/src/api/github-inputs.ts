import type { ConnectRepoGitHubInput, GitHubSetupInput, RepoGitHubAuthorizeInput } from './types'
import { parseRepoParams } from './repo-params'

const SETUP_INCOMPLETE = 'GitHub did not send everything needed to finish connecting. Start again from repository settings.'

export function parseGitHubSetupInput(input: unknown): GitHubSetupInput {
  const data = input as Partial<GitHubSetupInput> | null
  return {
    state: requiredText(data?.state, SETUP_INCOMPLETE),
    code: requiredText(data?.code, SETUP_INCOMPLETE),
  }
}

export function parseRepoGitHubAuthorizeInput(input: unknown): RepoGitHubAuthorizeInput {
  const params = parseRepoParams(input)
  const data = input as Partial<RepoGitHubAuthorizeInput>
  return {
    ...params,
    web_origin: typeof data.web_origin === 'string' && data.web_origin ? data.web_origin : null,
  }
}

export function parseConnectRepoGitHubInput(input: unknown): ConnectRepoGitHubInput {
  const params = parseRepoParams(input)
  const data = input as Partial<ConnectRepoGitHubInput>
  return {
    ...params,
    grant: requiredText(data.grant, SETUP_INCOMPLETE),
    github_repository_id: positiveId(data.github_repository_id, 'Choose a GitHub repository.'),
  }
}

function requiredText(value: unknown, message: string) {
  const text = typeof value === 'string' ? value.trim() : ''
  if (!text) throw new Error(message)
  return text
}

function positiveId(value: unknown, message: string) {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value <= 0) {
    throw new Error(message)
  }
  return value
}
