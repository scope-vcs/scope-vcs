import type { RequestAuthorRecordResponse } from '../../../api/types.generated'

type AuthorRecordTokenKind = 'merged' | 'closed' | 'open' | 'rating' | 'first'
export type AuthorRecordMark = AuthorRecordTokenKind | 'withdrawn' | 'unrated'

export type AuthorRecordToken = {
  kind: AuthorRecordTokenKind
  value: string | null
  tip: string
}

export type AuthorRecordSummary = {
  mark: 'rating' | 'unrated' | 'first'
  value: string | null
  label: string
  lines: { mark: AuthorRecordMark; value: string | null; text: string }[]
}

const FIRST_REQUEST = 'First request in this repository'

function averageRating({ count, score_sum: scoreSum }: { count: number; score_sum: number }) {
  return (scoreSum / count).toFixed(1)
}

export function authorRecordTokens(record: RequestAuthorRecordResponse): AuthorRecordToken[] {
  if (record.kind === 'first_request') return [{ kind: 'first', value: null, tip: FIRST_REQUEST }]
  const counts = [
    { kind: 'merged', count: record.merged, label: 'merged' },
    { kind: 'closed', count: record.closed, label: 'closed by a maintainer' },
    { kind: 'open', count: record.other_open, label: record.other_open === 1 ? 'other request open' : 'other requests open' },
  ] as const
  const tokens: AuthorRecordToken[] = counts
    .filter(({ count }) => count > 0)
    .map(({ kind, count, label }) => ({ kind, value: String(count), tip: `${count} ${label}` }))
  if (record.rating) {
    const average = averageRating(record.rating)
    const { count } = record.rating
    tokens.push({
      kind: 'rating',
      value: average,
      tip: `Rated ${average} by maintainers here, from ${count} ${count === 1 ? 'rating' : 'ratings'}`,
    })
  }
  return tokens
}

export function authorRecordSummary(record: RequestAuthorRecordResponse): AuthorRecordSummary {
  if (record.kind === 'first_request') {
    return { mark: 'first', value: null, label: FIRST_REQUEST, lines: [{ mark: 'first', value: null, text: FIRST_REQUEST }] }
  }
  const counts = [
    { mark: 'merged', count: record.merged, text: 'merged' },
    { mark: 'closed', count: record.closed, text: 'closed' },
    { mark: 'withdrawn', count: record.withdrawn, text: 'withdrawn' },
    { mark: 'open', count: record.other_open, text: record.other_open === 1 ? 'other open' : 'others open' },
  ] as const
  const lines: AuthorRecordSummary['lines'] = counts
    .filter(({ count }) => count > 0)
    .map(({ mark, count, text }) => ({ mark, value: String(count), text }))
  if (!record.rating) {
    return { mark: 'unrated', value: null, label: 'Not rated yet', lines: [...lines, { mark: 'unrated', value: null, text: 'not rated yet' }] }
  }
  const average = averageRating(record.rating)
  const { count } = record.rating
  return {
    mark: 'rating',
    value: average,
    label: `Rated ${average}`,
    lines: [...lines, { mark: 'rating', value: average, text: `from ${count} ${count === 1 ? 'rating' : 'ratings'}` }],
  }
}
