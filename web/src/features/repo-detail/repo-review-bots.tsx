import type { GitHubConnectionResponse, GitHubReviewBotResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { useEffect, useState } from 'react'
import {
  reviewBotLogins,
  reviewBotsProblem,
  reviewBotsText,
  sameReviewBots,
} from './repo-review-bots-model'

function sameSaved(saved: GitHubReviewBotResponse[] | null, bots: GitHubReviewBotResponse[]) {
  return saved !== null && sameReviewBots(saved.map((bot) => bot.login), bots)
}

export function RepoReviewBots({
  github,
  save,
}: {
  github: GitHubConnectionResponse
  save: (logins: string[]) => Promise<GitHubConnectionResponse>
}) {
  const bots = github.review_bots
  const [draft, setDraft] = useState(() => reviewBotsText(bots))
  const [source, setSource] = useState(bots)
  const [pending, setPending] = useState(false)
  const [saved, setSaved] = useState<GitHubReviewBotResponse[] | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const logins = reviewBotLogins(draft)
  const dirty = !sameReviewBots(logins, bots)

  useEffect(() => {
    if (bots === source) return
    if (sameReviewBots(reviewBotLogins(draft), source)) setDraft(reviewBotsText(bots))
    setSource(bots)
  }, [bots, draft, source])

  async function update() {
    const found = reviewBotsProblem(logins)
    setProblem(found)
    if (found) return
    setError(null)
    setPending(true)
    try {
      const result = await save(logins)
      setDraft(reviewBotsText(result.review_bots))
      setSource(result.review_bots)
      setSaved(result.review_bots)
    } catch (cause) {
      setError(resourceErrorMessage(cause, 'Try again.'))
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <label className="block leading-5 font-medium" htmlFor="github-review-bots">Review bots</label>
        <p className="leading-5 text-muted-foreground" id="github-review-bots-description">
          Scope imports GitHub reviews from these bots and from GitHub users who can write to the repository.
        </p>
      </div>
      <form
        className="space-y-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (dirty && !pending) void update()
        }}
      >
        <textarea
          aria-describedby={problem ? 'github-review-bots-problem' : 'github-review-bots-description'}
          aria-invalid={problem ? true : undefined}
          className={cn(
            'block min-h-20 w-full resize-y rounded-lg border border-input bg-secondary px-3 py-2 font-mono text-[13px] leading-5 shadow-[var(--shadow-card)]',
            'placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring',
            'disabled:cursor-not-allowed disabled:bg-muted disabled:opacity-50 aria-invalid:border-destructive',
          )}
          disabled={pending}
          id="github-review-bots"
          onChange={(event) => {
            setDraft(event.target.value)
            setProblem(null)
            setError(null)
          }}
          placeholder={'greptile-apps[bot]\nchatgpt-codex-connector[bot]'}
          rows={Math.min(Math.max(logins.length + 1, 3), 8)}
          spellCheck={false}
          value={draft}
        />
        {problem ? (
          <p className="leading-5 text-destructive" id="github-review-bots-problem" role="alert">
            {problem}
          </p>
        ) : null}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p aria-live="polite" className="leading-5 text-muted-foreground">
            {sameSaved(saved, bots) && !dirty ? 'Review bots saved.' : null}
          </p>
          <Button disabled={!dirty || pending} size="sm" type="submit">
            {pending ? 'Saving…' : 'Save review bots'}
          </Button>
        </div>
      </form>
      {error && (
        <PageErrorAlert className="mt-0" title="Review bots were not saved">
          {error}
        </PageErrorAlert>
      )}
    </div>
  )
}
