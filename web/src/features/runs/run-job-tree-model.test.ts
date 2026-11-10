import assert from 'node:assert/strict'
import test from 'node:test'
import {
  buildRunJobTree,
  filterRunJobTree,
  runJobGroupOpensByDefault,
  runJobTallies,
  type RunJobGroup,
  type RunJobNode,
} from './run-job-tree-model'

const job = (name: string, state = 'succeeded') => ({ key: name, name, state, startedAt: null, completedAt: null })
const jobs = [
  job('Plan selected components'),
  job('Validate selected components / Server validation / Web validation / Check scope-web', 'failed'),
  job('Validate selected components / Server validation / Server validation gate'),
  job('Validate selected components / Client validation / Check CLI'),
  job('Validate selected components / Client validation / Check desktop'),
  job('Validate selected components / Docs / Check links'),
  job('Release / publish', 'skipped'),
]

function outline(nodes: readonly RunJobNode[]): unknown[] {
  return nodes.map((node) => node.kind === 'job' ? node.label : [node.label, node.jobs.length, outline(node.children)])
}

test('job names group on " / " two levels deep and a group of one job folds into its name', () => {
  const tree = buildRunJobTree(jobs)
  assert.deepEqual(outline(tree), [
    'Plan selected components',
    ['Validate selected components', 5, [
      ['Server validation', 2, ['Web validation / Check scope-web', 'Server validation gate']],
      ['Client validation', 2, ['Check CLI', 'Check desktop']],
      'Docs / Check links',
    ]],
    'Release / publish',
  ])
  const validate = tree[1] as RunJobGroup
  const client = validate.children[1] as RunJobGroup
  assert.equal(runJobGroupOpensByDefault(validate, null), true, 'a group holding a failed job opens')
  assert.equal(runJobGroupOpensByDefault(client, null), false)
  assert.equal(runJobGroupOpensByDefault(client, 'Validate selected components / Client validation / Check CLI'), true)
  assert.deepEqual(outline(filterRunJobTree(tree, ' check cli ')), [['Validate selected components', 1, [['Client validation', 1, ['Check CLI']]]]])
})

test('header tallies count jobs by outcome and leave out empty counts', () => {
  assert.deepEqual(
    runJobTallies(jobs).map(({ count, label }) => `${count} ${label}`),
    ['1 failed', '5 passed', '1 skipped'],
  )
})
