import type { GitHubConnectionDetailsResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { LoaderCircle } from 'lucide-react'
import { useId, useState } from 'react'
import { REVIEW_REQUEST_COMMENT_HINT, reviewRequestCommentDraft } from './repo-github-review-bots-model'
import { useSettingsAction } from './use-settings-action'

export function RepoGitHubReviewBots({
  connection,
  saveComment,
  setEnabled,
}: {
  connection: GitHubConnectionDetailsResponse
  saveComment: (comment: string) => Promise<unknown>
  setEnabled: (enabled: boolean) => Promise<unknown>
}) {
  const [draft, setDraft] = useState(connection.review_request_comment ?? '')
  const { act, errorAlert, pending } = useSettingsAction<'toggle' | 'save'>()
  const switchId = useId()
  const commentId = useId()
  const comment = reviewRequestCommentDraft(draft, connection.review_request_comment)
  const enabled = connection.open_review_bot_pull_requests
  const canSave = comment.changed && !comment.tooLong && pending === null

  return (
    <div className="space-y-3 border-t border-border pt-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <label className="leading-5" htmlFor={switchId}>Open pull requests for review bots</label>
          <p className="leading-5 text-muted-foreground">
            Scope opens a GitHub pull request for each submitted full-view request so review bots
            on GitHub review it, and closes it when the request ends.
          </p>
        </div>
        <Switch
          checked={enabled}
          disabled={pending !== null}
          id={switchId}
          onCheckedChange={(checked) =>
            void act('toggle', 'The setting was not saved', () => setEnabled(checked))}
        />
      </div>
      <form
        className="space-y-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (canSave) void act('save', 'The comment was not saved', () => saveComment(draft))
        }}
      >
        <div>
          <label className="leading-5" htmlFor={commentId}>Review request comment</label>
          <p className="leading-5 text-muted-foreground">
            Scope posts this on the pull request when it opens it and after each new push; leave
            it empty to post nothing.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            aria-invalid={comment.tooLong}
            className="h-8 min-w-0 flex-1"
            disabled={pending !== null}
            id={commentId}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="No comment"
            value={draft}
          />
          <Button disabled={!canSave} size="sm" type="submit" variant="secondary">
            {pending === 'save' && <LoaderCircle className="size-3.5 animate-spin" />}
            <span>Save comment</span>
          </Button>
        </div>
        {comment.tooLong && <p className="leading-5 text-danger-strong">{REVIEW_REQUEST_COMMENT_HINT}</p>}
      </form>
      {errorAlert}
    </div>
  )
}
