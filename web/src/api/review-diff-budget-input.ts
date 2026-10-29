import type { ReviewDiffRenderBudget } from './types'

export function parseReviewDiffRenderBudget(value: unknown): ReviewDiffRenderBudget {
  if (value === 'full' || value === 'standard') return value
  throw new Error('budget must be full or standard.')
}
