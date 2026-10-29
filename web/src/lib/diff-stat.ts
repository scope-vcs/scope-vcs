import type { CommitFileResponse, FileLineChangesResponse, RequestDiffStatResponse } from '@/api/types.generated'

export type DiffStat = RequestDiffStatResponse

const ABBREVIATIONS = [
  { size: 1_000_000, suffix: 'm' },
  { size: 1_000, suffix: 'k' },
] as const

const EXACT_COUNT = new Intl.NumberFormat('en-US')

export function abbreviatedCount(count: number): string {
  for (const { size, suffix } of ABBREVIATIONS) {
    if (count < size) continue
    const shown = count < size * 100
      ? Math.floor((count * 10) / size) / 10
      : Math.floor(count / size)
    return `${shown}${suffix}`
  }
  return String(count)
}

export function exactCount(count: number): string {
  return EXACT_COUNT.format(count)
}

export function diffStatWithChanges(stat: DiffStat | null): DiffStat | null {
  return stat && stat.changed_files > 0 ? stat : null
}

export function diffStatHasLines({ additions, deletions }: DiffStat): boolean {
  return additions + deletions > 0
}

export function diffStatOfFiles(files: readonly CommitFileResponse[]): DiffStat {
  return files.reduce<DiffStat>(
    (stat, { lines }) => lines.kind === 'text'
      ? { ...stat, additions: stat.additions + lines.additions, deletions: stat.deletions + lines.deletions }
      : stat,
    { additions: 0, deletions: 0, changed_files: files.length },
  )
}

export function diffStatFilesLabel(changedFiles: number): string {
  return counted(changedFiles, 'file')
}

export function diffStatLabel(stat: DiffStat): string {
  const files = diffStatFilesLabel(stat.changed_files)
  if (!diffStatHasLines(stat)) return `${files} changed, no line changes`
  return `${lineChangesLabel(stat.additions, stat.deletions)} across ${files}`
}

export function fileLinesLabel(lines: FileLineChangesResponse): string {
  return lines.kind === 'binary' ? 'Binary file' : lineChangesLabel(lines.additions, lines.deletions)
}

function lineChangesLabel(additions: number, deletions: number) {
  return `${counted(additions, 'addition')}, ${counted(deletions, 'deletion')}`
}

function counted(count: number, noun: string) {
  return `${exactCount(count)} ${count === 1 ? noun : `${noun}s`}`
}
