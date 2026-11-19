import type { RequestAuthorRecordResponse } from '../../../api/types.generated'

export type AuthorRecordTokenKind = 'merged' | 'closed' | 'withdrawn' | 'open' | 'rating' | 'first'

export type AuthorRecordToken = {
  kind: AuthorRecordTokenKind
  value: string | null
  ratingCount: number | null
  tip: string
}

export function authorRecordTokens(record: RequestAuthorRecordResponse, detailed: boolean): AuthorRecordToken[] {
  if (record.kind === 'first_request') {
    return [{ kind: 'first', value: null, ratingCount: null, tip: 'First request in this repository' }]
  }
  const counts = [
    { kind: 'merged', count: record.merged, label: 'merged' },
    { kind: 'closed', count: record.closed_by_maintainer, label: 'closed by a maintainer' },
    { kind: 'withdrawn', count: record.withdrawn, label: 'withdrawn by the author' },
    { kind: 'open', count: record.other_open, label: record.other_open === 1 ? 'other request open' : 'other requests open' },
  ] as const
  const tokens: AuthorRecordToken[] = counts
    .filter(({ count, kind }) => count > 0 && (detailed || kind !== 'withdrawn'))
    .map(({ kind, count, label }) => ({ kind, value: String(count), ratingCount: null, tip: `${count} ${label}` }))
  if (record.rating) {
    const { count, score_sum: scoreSum } = record.rating
    const average = (scoreSum / count).toFixed(1)
    tokens.push({
      kind: 'rating',
      value: average,
      ratingCount: count,
      tip: `Rated ${average} by maintainers here, from ${count} ${count === 1 ? 'rating' : 'ratings'}`,
    })
  }
  return tokens
}

export function authorRecordSummary(record: RequestAuthorRecordResponse) {
  return authorRecordTokens(record, true).map(({ tip }) => tip).join(', ')
}
