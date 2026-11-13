import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubCatalogCheckResponse,
  GitHubCheckRunsResponse,
  GitHubWorkflowSummaryResponse,
} from '../../api/types.generated'
import { githubCheckStatus, githubWorkflowsView } from './repo-github-checks-model'

const YES: GitHubCheckRunsResponse = { verdict: 'yes', reason: null }
const NO: GitHubCheckRunsResponse = { verdict: 'no', reason: 'No trigger for Scope branches.' }
const MAYBE: GitHubCheckRunsResponse = { verdict: 'maybe', reason: 'Runs only when its condition holds.' }

function workflow(name: string, runs: GitHubCheckRunsResponse, runtime: string[] = []): GitHubWorkflowSummaryResponse {
  return { path: `.github/workflows/${name}.yml`, name, runs, runtime_named_jobs: runtime }
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

test('workflows that do not run on requests ask for the trigger', () => {
  for (const [workflows, summary, attention, needsTrigger] of [
    [[], 'No workflows on main. Add one with this trigger so requests get checks.', [], true],
    [[workflow('ci', YES)], 'The workflow on main runs on Scope requests.', [], false],
    [
      [workflow('ci', YES), workflow('deploy', MAYBE), workflow('release', NO)],
      '1 of 3 workflows on main run on Scope requests.',
      ['deploy', 'release'],
      true,
    ],
    [[workflow('ci', YES), workflow('deploy', MAYBE)], '1 of 2 workflows on main run on Scope requests.', ['deploy'], false],
  ] as const) {
    const view = githubWorkflowsView({ workflows: [...workflows], checks: [] })
    assert.equal(view.summary, summary)
    assert.deepEqual(view.attention.map((item) => item.name), attention)
    assert.equal(view.needsTrigger, needsTrigger)
  }
  const runtime = githubWorkflowsView({ workflows: [workflow('ci', YES, ['build', 'test'])], checks: [] })
  assert.deepEqual(runtime.runtimeNamed, ['ci (build, test)'])
})

test('each result says whether it runs on requests and where else it comes from', () => {
  for (const [entry, text, tone] of [
    [undefined, 'Not found in workflows or on GitHub', 'warning'],
    [check({ runs: YES, required_on_github: true }), 'Runs on Scope requests, required on GitHub', 'muted'],
    [check({ runs: MAYBE, required_on_github: true }), 'Runs only when its condition holds, required on GitHub', 'muted'],
    [check({ runs: NO }), 'Does not run on Scope requests', 'warning'],
    [check({ required_on_github: true }), 'Required on GitHub, reported by another app', 'muted'],
    [check({ reported_on_main: true }), 'Reported on main by another app', 'muted'],
  ] as const) {
    assert.deepEqual(githubCheckStatus(entry), { text, tone })
  }
})
