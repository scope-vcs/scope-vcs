import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubCatalogCheckResponse,
  GitHubCheckRunsResponse,
  GitHubWorkflowSummaryResponse,
} from '../../api/types.generated'
import { githubCheckStatus, githubWorkflowsView, searchWorkflows, workflowFileUrl } from './repo-github-checks-model'

const YES: GitHubCheckRunsResponse = { verdict: 'yes', reason: null }
const NO: GitHubCheckRunsResponse = { verdict: 'no', reason: 'No trigger for Scope branches.' }
const DISABLED: GitHubCheckRunsResponse = { verdict: 'no', reason: 'Disabled on GitHub.' }
const MAYBE: GitHubCheckRunsResponse = { verdict: 'maybe', reason: 'Runs only when its condition holds.' }

function workflow(
  name: string,
  runs: GitHubCheckRunsResponse,
  runtime: string[] = [],
  triggers: string[] = ['push'],
): GitHubWorkflowSummaryResponse {
  return { path: `.github/workflows/${name}.yml`, name, runs, triggers, runtime_named_jobs: runtime }
}

function check(overrides: Partial<GitHubCatalogCheckResponse>): GitHubCatalogCheckResponse {
  return {
    name: 'test',
    workflow_path: null,
    runs: null,
    required_on_github: false,
    reported_on_main: false,
    ...overrides,
  }
}

test('workflows group by outcome, most urgent first, and search by path, name or trigger', () => {
  for (const [workflows, summary, groups] of [
    [[], 'No workflows on main. Add one with this trigger so requests get checks.', []],
    [[workflow('ci', YES)], 'The workflow on main runs on Scope requests.', [['yes', ['ci']]]],
    [[workflow('release', NO)], '0 of 1 workflow on main run on Scope requests.', [['no', ['release']]]],
    [
      [workflow('ci', YES), workflow('deploy', MAYBE), workflow('nightly', NO), workflow('old', DISABLED), workflow('release', NO)],
      '1 of 5 workflows on main run on Scope requests.',
      [['no', ['nightly', 'release']], ['no', ['old']], ['maybe', ['deploy']], ['yes', ['ci']]],
    ],
  ] as const) {
    const view = githubWorkflowsView({ workflows: [...workflows], checks: [] })
    assert.equal(view.summary, summary)
    assert.deepEqual(
      view.groups.map((group) => [group.verdict, group.workflows.map((item) => item.name)]),
      groups,
    )
  }
  const runtime = githubWorkflowsView({ workflows: [workflow('ci', YES, ['build', 'test'])], checks: [] })
  assert.deepEqual(runtime.runtimeNamed, ['ci (build, test)'])

  const { groups } = githubWorkflowsView({
    checks: [],
    workflows: [
      workflow('ci', YES),
      { ...workflow('nightly', NO, [], ['schedule', 'workflow_dispatch']), name: 'Nightly Backup' },
      workflow('release', NO, [], ['workflow_dispatch']),
    ],
  })
  for (const [query, names] of [
    ['  DISPATCH ', ['Nightly Backup', 'release']],
    ['backup', ['Nightly Backup']],
    ['ci.yml', ['ci']],
    ['deploy', []],
  ] as const) {
    assert.deepEqual(searchWorkflows(groups, query).map((item) => item.name), names, query)
  }
})

test('each result says whether it runs on requests and where else it comes from', () => {
  for (const [entry, text, tone] of [
    [undefined, 'Not found in workflows or on GitHub', 'warning'],
    [check({ runs: YES, required_on_github: true }), 'Runs on Scope requests, required on GitHub', 'muted'],
    [check({ runs: MAYBE, required_on_github: true }), 'Runs only when its condition holds, required on GitHub', 'muted'],
    [check({ runs: NO }), 'Does not run on Scope requests', 'warning'],
    [check({ required_on_github: true, reported_on_main: true }), 'Required on GitHub, reported by another app', 'muted'],
    [check({ required_on_github: true }), 'Required on GitHub but not reported as a check run, so Scope cannot wait for it', 'warning'],
    [check({ reported_on_main: true }), 'Reported on main by another app', 'muted'],
  ] as const) {
    assert.deepEqual(githubCheckStatus(entry), { text, tone })
  }
})

test('workflow links open main and keep reserved characters inside the file name', () => {
  assert.equal(
    workflowFileUrl('https://github.com/octo/demo', '.github/workflows/ci#nightly?.yml'),
    'https://github.com/octo/demo/blob/main/.github/workflows/ci%23nightly%3F.yml',
  )
})
