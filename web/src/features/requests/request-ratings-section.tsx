import type { RequestParams, RequestRating, RequestRatings } from '@/api/types'
import type { RateRequestInput } from '@/api/requests'
import { Button } from '@/components/ui/button'
import { Star } from 'lucide-react'
import { type FormEvent, useState } from 'react'

type FormState = {
  error: string | null
  reason: string
  score: number
  submitting: boolean
}

const initialFormState: FormState = {
  error: null,
  reason: '',
  score: 5,
  submitting: false,
}

export function RequestRatingsSection({
  initial,
  onRate,
  params,
}: {
  initial: RequestRatings
  onRate: (input: RateRequestInput) => Promise<RequestRating>
  params: RequestParams
}) {
  const [{ error, reason, score, submitting }, setForm] = useState(initialFormState)
  const { eligible_subject: eligibleSubject, ratings } = initial

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!eligibleSubject || submitting) return
    setForm(current => ({ ...current, error: null, submitting: true }))
    try {
      await onRate({ ...params, reason, score })
      setForm(current => ({ ...current, reason: '', submitting: false }))
    } catch (cause) {
      setForm(current => ({
        ...current,
        error: cause instanceof Error ? cause.message : 'Could not submit rating.',
        submitting: false,
      }))
    }
  }

  return (
    <details open={ratings.length > 0 || Boolean(eligibleSubject) ? true : undefined}>
      <summary className="cursor-pointer list-item items-center gap-2 text-[13px] font-semibold text-muted-foreground">
        <Star className="mr-2 inline size-3.5" />
        <h2 className="inline">participant ratings</h2>
      </summary>
      {ratings.length ? (
        <div className="mt-3 divide-y divide-border">
          {ratings.map((rating) => (
            <div className="py-3 text-xs leading-5" key={rating.id}>
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
      ) : (
        <p className="mt-3 text-xs leading-5 text-muted-foreground">No participant ratings yet.</p>
      )}

      {eligibleSubject ? (
        <form className="mt-4 grid gap-3 border-t border-border pt-4" onSubmit={submit}>
          <label className="grid gap-1 text-xs font-medium">
            Rating for @{eligibleSubject.handle}
            <select
              className="h-9 rounded-md border border-input bg-background px-2 text-sm"
              disabled={submitting}
              onChange={(event) => setForm(current => ({ ...current, score: Number(event.target.value) }))}
              value={score}
            >
              {[5, 4, 3, 2, 1].map((value) => (
                <option key={value} value={value}>{value} / 5</option>
              ))}
            </select>
          </label>
          <label className="grid gap-1 text-xs font-medium">
            Reason
            <textarea
              className="min-h-24 resize-y rounded-md border border-input bg-background px-3 py-2 text-sm font-normal"
              disabled={submitting}
              maxLength={1024}
              onChange={(event) => setForm(current => ({ ...current, reason: event.target.value }))}
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
    </details>
  )
}
