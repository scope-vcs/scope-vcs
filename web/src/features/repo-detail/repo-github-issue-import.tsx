import type { RepoViews } from '@/api/repo-views'
import type { GitHubConnectionResponse, GitHubIssueImportResponse, ViewId } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { ViewSelect } from '@/components/view-select'
import { cn } from '@/lib/utils'
import { Download, LoaderCircle } from 'lucide-react'
import { useId, useState } from 'react'
import { githubIssueImportView } from './repo-github-issue-import-model'
import { useSettingsAction } from './use-settings-action'

export function RepoGitHubIssueImport({
  defaultView,
  importInto,
  issueImport,
  views,
}: {
  defaultView: ViewId
  importInto: (view: ViewId) => Promise<GitHubConnectionResponse>
  issueImport: GitHubIssueImportResponse | null
  views: Pick<RepoViews, 'definitions' | 'name'>
}) {
  const viewFieldId = useId()
  const [view, setView] = useState(defaultView)
  const { act, errorAlert, pending } = useSettingsAction<'import'>()
  const status = githubIssueImportView(issueImport, issueImport ? views.name(issueImport.view) : '')
  const working = pending !== null || status?.working === true

  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <div className="leading-5">Import open issues</div>
        <p className="leading-5 text-muted-foreground">
          Scope opens a request in the view you choose for each open issue on GitHub, with its
          comments. Issues that already have a request stay as they are. Nothing is written to
          GitHub.
        </p>
      </div>
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (!working) void act('import', 'The import could not start', () => importInto(view))
        }}
      >
        <label className="text-muted-foreground" htmlFor={viewFieldId}>View</label>
        <ViewSelect
          className="max-w-48"
          disabled={working}
          id={viewFieldId}
          onChange={setView}
          value={view}
          views={views.definitions}
        />
        <Button disabled={working} size="sm" type="submit" variant="secondary">
          {working ? <LoaderCircle className="size-3.5 animate-spin" /> : <Download className="size-3.5" />}
          <span>Import open issues</span>
        </Button>
      </form>
      {status && (
        <p aria-live="polite" className="break-words leading-5 text-muted-foreground">
          <span className={cn(status.failed && 'text-danger-strong')}>{status.status}</span>
          {status.read && ` ${status.read}`}
        </p>
      )}
      {errorAlert}
    </div>
  )
}
