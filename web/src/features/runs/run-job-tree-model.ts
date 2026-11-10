import { runStatus, type RunTone } from './run-status'

export type RunJobSummary = {
  key: string
  name: string
  state: string
  startedAt: number | null
  completedAt: number | null
}

export type RunJobLeaf = { kind: 'job'; label: string; job: RunJobSummary }

export type RunJobGroup = {
  kind: 'group'
  path: string
  label: string
  children: RunJobNode[]
  jobs: RunJobSummary[]
}

export type RunJobNode = RunJobLeaf | RunJobGroup

const NAME_SEPARATOR = ' / '
const GROUP_DEPTH = 2

export function buildRunJobTree(jobs: readonly RunJobSummary[]): RunJobNode[] {
  const root: RunJobGroup = { kind: 'group', path: '', label: '', children: [], jobs: [] }
  for (const job of jobs) {
    const segments = job.name.split(NAME_SEPARATOR)
    const depth = Math.min(GROUP_DEPTH, segments.length - 1)
    let parent = root
    parent.jobs.push(job)
    for (const segment of segments.slice(0, depth)) {
      const path = parent.path ? `${parent.path}${NAME_SEPARATOR}${segment}` : segment
      let group = parent.children.find((node): node is RunJobGroup => node.kind === 'group' && node.path === path)
      if (!group) {
        group = { kind: 'group', path, label: segment, children: [], jobs: [] }
        parent.children.push(group)
      }
      group.jobs.push(job)
      parent = group
    }
    parent.children.push({ kind: 'job', label: segments.slice(depth).join(NAME_SEPARATOR), job })
  }
  return root.children.map(collapseSingleChild)
}

export function runJobShortName(name: string) {
  return name.split(NAME_SEPARATOR).at(-1) ?? name
}

export function filterRunJobTree(nodes: readonly RunJobNode[], query: string): RunJobNode[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return [...nodes]
  return nodes.flatMap((node): RunJobNode[] => {
    if (node.kind === 'job') return node.job.name.toLowerCase().includes(needle) ? [node] : []
    const children = filterRunJobTree(node.children, needle)
    return children.length ? [{ ...node, children, jobs: children.flatMap(nodeJobs) }] : []
  })
}

export function runJobGroupOpensByDefault(group: RunJobGroup, selectedKey: string | null) {
  return group.jobs.some((job) => job.key === selectedKey || jobNeedsAttention(job.state))
}

export function failedJobCount(jobs: readonly RunJobSummary[]) {
  return jobs.filter((job) => runStatus(job.state).tone === 'danger').length
}

const TALLY_LABELS: [RunTone, string][] = [
  ['danger', 'failed'],
  ['running', 'running'],
  ['waiting', 'waiting'],
  ['success', 'passed'],
  ['inert', 'skipped'],
]

export function runJobTallies(jobs: readonly Pick<RunJobSummary, 'state'>[]) {
  return TALLY_LABELS
    .map(([tone, label]) => ({ tone, label, count: jobs.filter((job) => runStatus(job.state).tone === tone).length }))
    .filter((tally) => tally.count > 0)
}

function jobNeedsAttention(state: string) {
  const { tone } = runStatus(state)
  return tone === 'danger' || tone === 'running'
}

function nodeJobs(node: RunJobNode): RunJobSummary[] {
  return node.kind === 'job' ? [node.job] : node.jobs
}

function collapseSingleChild(node: RunJobNode): RunJobNode {
  if (node.kind === 'job') return node
  const children = node.children.map(collapseSingleChild)
  if (children.length !== 1) return { ...node, children }
  const [only] = children as [RunJobNode]
  return { ...only, label: `${node.label}${NAME_SEPARATOR}${only.label}` }
}
