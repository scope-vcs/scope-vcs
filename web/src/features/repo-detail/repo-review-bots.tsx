import type { GitHubConnectionResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { useEffect, useState } from 'react'
import { reviewBotLogins, reviewBotsText, sameReviewBots } from './repo-review-bots-model'

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
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const dirty = !sameReviewBots(draft, bots)

  useEffect(() => {
    if (bots === source) return
    if (sameReviewBots(draft, source)) setDraft(reviewBotsText(bots))
    setSource(bots)
  }, [bots, draft, source])

  async function update() {
    setError(null)
    setPending(true)
    try {
      const result = await save(reviewBotLogins(draft))
      setDraft(reviewBotsText(result.review_bots))
      setSource(result.review_bots)
      setSaved(true)
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
        className="flex flex-wrap gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (dirty && !pending) void update()
        }}
      >
        <Input
          aria-describedby="github-review-bots-description"
          className="h-8 min-w-0 flex-1 font-mono text-[13px]"
          disabled={pending}
          id="github-review-bots"
          onChange={(event) => {
            setDraft(event.target.value)
            setSaved(false)
            setError(null)
          }}
          placeholder="greptile-apps[bot], chatgpt-codex-connector[bot]"
          value={draft}
        />
        <Button disabled={!dirty || pending} size="sm" type="submit">
          {pending ? 'Saving…' : 'Save review bots'}
        </Button>
      </form>
      {saved && !dirty ? (
        <p aria-live="polite" className="leading-5 text-muted-foreground">Review bots saved.</p>
      ) : null}
      {error && (
        <PageErrorAlert className="mt-0" title="Review bots were not saved">
          {error}
        </PageErrorAlert>
      )}
    </div>
  )
}
