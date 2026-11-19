import type { RequestParams } from '@/api/types'
import type {
  RequestRatingResponse,
  RequestRatingsResponse,
} from '@/api/types.generated'
import type { RateRequestInput } from '@/api/requests'
import { Button } from '@/components/ui/button'
import { Select } from '@/components/ui/select'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { type Dispatch, type SubmitEvent, useReducer, useState } from 'react'
import { DetailsRow, DetailsToggle } from './request-details-layout'

type FormState = {
  error: string | null
  reason: string
  score: number
  submitting: boolean
}

type FormAction =
  | { type: 'score_changed'; score: number }
  | { type: 'reason_changed'; reason: string }
  | { type: 'submission_started' }
  | { type: 'submission_succeeded' }
  | { type: 'submission_failed'; error: string }

const initialFormState: FormState = {
  error: null,
  reason: '',
  score: 5,
  submitting: false,
}

function formReducer(state: FormState, action: FormAction): FormState {
  switch (action.type) {
    case 'score_changed': return { ...state, score: action.score }
    case 'reason_changed': return { ...state, reason: action.reason }
    case 'submission_started': return { ...state, error: null, submitting: true }
    case 'submission_succeeded': return { ...state, reason: '', submitting: false }
    case 'submission_failed': return { ...state, error: action.error, submitting: false }
  }
}

export type RatingDraft = [FormState, Dispatch<FormAction>]

export function useRatingDraft(): RatingDraft {
  return useReducer(formReducer, initialFormState)
}

export function RequestRatingsSection({
  draft: [{ error, reason, score, submitting }, dispatch],
  initial,
  onRate,
  params,
}: {
  draft: RatingDraft
  initial: RequestRatingsResponse
  onRate: (input: RateRequestInput) => Promise<RequestRatingResponse>
  params: RequestParams
}) {
  const [expanded, setExpanded] = useState(reason !== '')
  const { eligible_subject: eligibleSubject, ratings } = initial

  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!eligibleSubject || submitting) return
    dispatch({ type: 'submission_started' })
    try {
      await onRate({ ...params, reason, score })
      dispatch({ type: 'submission_succeeded' })
    } catch (cause) {
      dispatch({
        type: 'submission_failed',
        error: resourceErrorMessage(cause, 'Could not submit rating.'),
      })
    }
  }

  return (
    <>
      <DetailsRow
        action={eligibleSubject || ratings.length ? (
          <DetailsToggle expanded={expanded} label={eligibleSubject ? 'Rate' : 'Show'} onToggle={() => setExpanded((value) => !value)} />
        ) : null}
        label="Ratings"
      >
        {ratings.length ? `${ratings.length} ${ratings.length === 1 ? 'rating' : 'ratings'}` : 'None yet'}
      </DetailsRow>
      {expanded ? (
        <>
          {ratings.length ? (
            <div className="divide-y divide-border">
              {ratings.map((rating) => (
                <div className="py-2 leading-5" key={rating.id}>
                  <div className="font-medium">
                    @{rating.rater.handle} rated @{rating.subject.handle} {rating.score}/5
                  </div>
                  <div className="text-muted-foreground">
                    @{rating.subject.handle}: {rating.subject.rating_count} ratings ·{' '}
                    {rating.subject.rating_score_sum} points
                  </div>
                  <p className="mt-1 whitespace-pre-wrap text-muted-foreground">{rating.reason}</p>
                </div>
              ))}
            </div>
          ) : null}
          {eligibleSubject ? (
            <form className="grid gap-2 py-2" onSubmit={(event) => void submit(event)}>
              <label className="grid gap-1 text-xs font-medium">
                Rating for @{eligibleSubject.handle}
                <Select
                  disabled={submitting}
                  onChange={(event) => dispatch({
                    type: 'score_changed',
                    score: Number(event.target.value),
                  })}
                  value={score}
                >
                  {[5, 4, 3, 2, 1].map((value) => (
                    <option key={value} value={value}>{value} / 5</option>
                  ))}
                </Select>
              </label>
              <label className="grid gap-1 text-xs font-medium">
                Reason
                <textarea
                  className="min-h-16 resize-y rounded-md border border-input bg-background px-2 py-1.5 text-xs font-normal"
                  disabled={submitting}
                  maxLength={1024}
                  onChange={(event) => dispatch({
                    type: 'reason_changed',
                    reason: event.target.value,
                  })}
                  required
                  value={reason}
                />
              </label>
              {error ? <p className="text-xs text-destructive" role="alert">{error}</p> : null}
              <Button disabled={submitting || !reason.trim()} size="sm" type="submit">
                {submitting ? 'Submitting…' : 'Submit rating'}
              </Button>
            </form>
          ) : null}
        </>
      ) : null}
    </>
  )
}
