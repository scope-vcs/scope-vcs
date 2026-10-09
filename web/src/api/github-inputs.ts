import type {
  ConnectRepoGitHubInput,
  GitHubSetupInput,
  RepoGitHubAuthorizeInput,
  RepoGitHubWorkflowJobLogInput,
  RepoGitHubWorkflowRunsInput,
  RunActionInput,
  SetRepoGitHubRequiredChecksInput,
  SetRepoGitHubRunImportCountInput,
} from './types'
import { parseRepoParams } from './repo-params'

export const GITHUB_RUN_IMPORT_MAX_COUNT = 1000

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
    acknowledge_public: data.acknowledge_public === true,
    run_import_count: runImportCount(data.run_import_count),
  }
}

export function parseSetRepoGitHubRunImportCountInput(input: unknown): SetRepoGitHubRunImportCountInput {
  const params = parseRepoParams(input)
  return { ...params, count: runImportCount((input as Partial<SetRepoGitHubRunImportCountInput>).count) }
}

export function parseRepoGitHubWorkflowRunsInput(input: unknown): RepoGitHubWorkflowRunsInput {
  const params = parseRepoParams(input)
  const data = input as Partial<RepoGitHubWorkflowRunsInput>
  return {
    ...params,
    workflow: optionalText(data.workflow, 'Choose a workflow.'),
    after: optionalText(data.after, 'Runs could not continue from there.'),
  }
}

export function parseRepoGitHubWorkflowRunInput(input: unknown): RunActionInput {
  const params = parseRepoParams(input)
  return { ...params, run_id: githubId((input as Partial<RunActionInput>).run_id, 'run_id') }
}

export function parseRepoGitHubWorkflowJobLogInput(input: unknown): RepoGitHubWorkflowJobLogInput {
  const run = parseRepoGitHubWorkflowRunInput(input)
  return { ...run, job_id: githubId((input as Partial<RepoGitHubWorkflowJobLogInput>).job_id, 'job_id') }
}

function githubId(value: unknown, label: string) {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) throw new Error(`${label} must be a GitHub id`)
  return value
}

export function isRunImportCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value)
    && value >= 0 && value <= GITHUB_RUN_IMPORT_MAX_COUNT
}

function runImportCount(value: unknown) {
  if (!isRunImportCount(value)) {
    throw new Error(`Choose between 0 and ${GITHUB_RUN_IMPORT_MAX_COUNT} recent runs to import.`)
  }
  return value
}

function optionalText(value: unknown, message: string) {
  if (value === undefined) return undefined
  if (typeof value !== 'string' || !value) throw new Error(message)
  return value
}

export function parseSetRepoGitHubRequiredChecksInput(
  input: unknown,
): SetRepoGitHubRequiredChecksInput {
  const params = parseRepoParams(input)
  const names = (input as Partial<SetRepoGitHubRequiredChecksInput>).names
  if (!Array.isArray(names) || names.some((name) => typeof name !== 'string')) {
    throw new Error('CI requirements must be a list of result names.')
  }
  return { ...params, names }
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
