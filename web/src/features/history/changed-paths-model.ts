import type { VisibilityState } from '@/api/types'
import type {
  CommitFileResponse,
  FileChangeKind,
  HistoryEntryDetailResponse,
} from '@/api/types.generated'
import { normalizeFilePath } from '../../components/file-system-tree-model'

export type HistoryVisibilityChange = HistoryEntryDetailResponse['visibility_changes'][number]

export type ChangedPath = {
  path: string
  group: string
  directory: string
  name: string
  file: CommitFileResponse | null
  visibility: HistoryVisibilityChange[]
}

export type ChangedPathFilter = 'all' | 'visibility' | FileChangeKind

export type ChangedPathTarget =
  | { kind: 'file'; file: CommitFileResponse }
  | { kind: 'visibility'; change: HistoryVisibilityChange }

export type ChangedPathGroup = { name: string; paths: ChangedPath[] }

const FILTER_ORDER: ChangedPathFilter[] = ['all', 'visibility', 'Added', 'Modified', 'Deleted']
const SMALL_CHANGE_LIMIT = 20

export function changedPaths(
  files: readonly CommitFileResponse[],
  changes: readonly HistoryVisibilityChange[] = [],
): ChangedPath[] {
  const byPath = new Map<string, ChangedPath>()
  const entry = (rawPath: string) => {
    const path = normalizeFilePath(rawPath)
    let found = byPath.get(path)
    if (!found) {
      found = { ...pathParts(path), file: null, visibility: [] }
      byPath.set(path, found)
    }
    return found
  }
  for (const file of files) entry(file.path).file = file
  for (const change of changes) entry(change.path).visibility.push(change)
  return [...byPath.values()].sort(compareChangedPaths)
}

export function changedPathFilterCounts(paths: readonly ChangedPath[]) {
  return FILTER_ORDER
    .map((filter) => ({ filter, count: paths.filter((path) => matchesFilter(path, filter)).length }))
    .filter(({ filter, count }) => filter === 'all' || count > 0)
}

export function filterChangedPaths(
  paths: readonly ChangedPath[],
  filter: ChangedPathFilter,
  query: string,
) {
  const needle = query.trim().toLowerCase()
  return paths.filter((path) =>
    matchesFilter(path, filter) && (!needle || path.path.toLowerCase().includes(needle)))
}

export function groupChangedPaths(paths: readonly ChangedPath[]) {
  const root: ChangedPath[] = []
  const groups: ChangedPathGroup[] = []
  for (const path of paths) {
    if (!path.group) {
      root.push(path)
      continue
    }
    const last = groups.at(-1)
    if (last?.name === path.group) last.paths.push(path)
    else groups.push({ name: path.group, paths: [path] })
  }
  return { root, groups }
}

export function initiallyOpenGroups(paths: readonly ChangedPath[], selectedPath: string | null) {
  const { groups } = groupChangedPaths(paths)
  if (paths.length <= SMALL_CHANGE_LIMIT || groups.length <= 1) {
    return new Set(groups.map((group) => group.name))
  }
  const selected = selectedPath === null ? undefined : findChangedPath(paths, selectedPath)
  return new Set(selected?.group ? [selected.group] : [])
}

export function visibilitySummary(paths: readonly ChangedPath[]) {
  const labels = paths.flatMap((path) => path.visibility.at(-1)?.new_label ?? [])
  if (labels.length === 0) return null
  const label: VisibilityState = labels.every((label) => label === labels[0]) ? labels[0] : 'Mixed'
  return { count: labels.length, label }
}

export function changedPathTargets(path: ChangedPath): ChangedPathTarget[] {
  return [
    ...(path.file ? [{ kind: 'file' as const, file: path.file }] : []),
    ...path.visibility
      .filter((change) => change.file !== null)
      .map((change) => ({ kind: 'visibility' as const, change })),
  ]
}

export function findChangedPath(paths: readonly ChangedPath[], rawPath: string) {
  const path = normalizeFilePath(rawPath)
  return paths.find((candidate) => candidate.path === path)
}

export function openableChangedPaths(paths: readonly ChangedPath[]) {
  return paths.filter(canOpenChangedPath)
}

export function canOpenChangedPath(path: ChangedPath) {
  return path.file !== null || path.visibility.some((change) => change.file !== null)
}

export function steppedChangedPath(
  openable: readonly ChangedPath[],
  current: number,
  step: 1 | -1,
) {
  if (openable.length === 0) return null
  if (current === -1) return step === 1 ? openable[0] : openable.at(-1)!
  return openable[Math.min(openable.length - 1, Math.max(0, current + step))]
}

function matchesFilter(path: ChangedPath, filter: ChangedPathFilter) {
  if (filter === 'all') return true
  if (filter === 'visibility') return path.visibility.length > 0
  return path.file?.kind === filter
}

function pathParts(path: string) {
  const parts = path.split('/')
  const name = parts.at(-1) ?? path
  if (parts.length === 1) return { path, group: '', directory: '', name }
  return { path, group: parts[0], directory: parts.slice(1, -1).join('/'), name }
}

function compareChangedPaths(left: ChangedPath, right: ChangedPath) {
  return compareText(left.group, right.group)
    || compareText(left.directory, right.directory)
    || compareText(left.name, right.name)
}

function compareText(left: string, right: string) {
  return left < right ? -1 : left > right ? 1 : 0
}
