import type { GitHubConnectionResponse } from '@/api/types.generated'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import { useId } from 'react'
import { githubMainPublicationView } from './repo-github-main-publication-model'
import { useSettingsAction } from './use-settings-action'

export function RepoGitHubMainPublication({
  github,
  setEnabled,
}: {
  github: GitHubConnectionResponse
  setEnabled: (enabled: boolean) => Promise<unknown>
}) {
  const { act, errorAlert, pending } = useSettingsAction<'publish'>()
  const blockedReasonId = useId()
  const view = githubMainPublicationView(github)
  if (!view) return null
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <label className="flex items-start justify-between gap-4">
        <span className="min-w-0">
          <span className="block leading-5">Publish main to GitHub</span>
          <span className="block leading-5 text-muted-foreground">
            After each merge or push to main, Scope pushes its main to GitHub’s main and never
            force-pushes.
          </span>
        </span>
        <Switch
          aria-describedby={view.blockedReason ? blockedReasonId : undefined}
          checked={view.enabled}
          disabled={pending !== null || view.blockedReason !== null}
          onCheckedChange={(enabled) =>
            void act('publish', 'Publishing could not change', () => setEnabled(enabled))}
          type="button"
        />
      </label>
      {view.blockedReason && (
        <p className="leading-5 text-muted-foreground" id={blockedReasonId}>{view.blockedReason}</p>
      )}
      {view.status && (
        <p aria-live="polite" className="leading-5 text-muted-foreground">{view.status}</p>
      )}
      {view.attempt && (
        <p
          className={cn(
            'break-words leading-5',
            view.attempt.failed ? 'text-danger-strong' : 'text-muted-foreground',
          )}
          role={view.attempt.failed ? 'alert' : undefined}
        >
          {view.attempt.text}
        </p>
      )}
      {errorAlert}
    </div>
  )
}
