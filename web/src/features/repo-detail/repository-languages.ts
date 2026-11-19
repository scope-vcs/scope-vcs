import type { RepoLanguageResponse } from '../../api/types.generated'

const NAMED_LANGUAGE_LIMIT = 6

export type LanguageSegment = {
  color: string | null
  name: string
  percent: string
  share: number
}

export function languageSegments(languages: readonly RepoLanguageResponse[]): LanguageSegment[] {
  const total = languages.reduce((bytes, language) => bytes + language.bytes, 0)
  if (total === 0) return []
  const named = languages.length > NAMED_LANGUAGE_LIMIT + 1
    ? languages.slice(0, NAMED_LANGUAGE_LIMIT)
    : languages
  const segments = named.map(({ bytes, color, name }) => segment(name, color, bytes / total))
  const otherBytes = languages.slice(named.length).reduce((bytes, language) => bytes + language.bytes, 0)
  return otherBytes > 0 ? [...segments, segment('Other', null, otherBytes / total)] : segments
}

export function languageSummary(segments: readonly LanguageSegment[]) {
  return `Languages: ${segments.map(({ name, percent }) => `${name} ${percent}`).join(', ')}`
}

function segment(name: string, color: string | null, share: number): LanguageSegment {
  return { color, name, percent: percentLabel(share), share }
}

function percentLabel(share: number) {
  const percent = share * 100
  return percent > 0 && percent < 0.1 ? '<0.1%' : `${percent.toFixed(1)}%`
}
