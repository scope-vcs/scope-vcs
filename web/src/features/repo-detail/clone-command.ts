import type { RepositoryActor, ViewId } from '../../api/types.generated'

export type CloneCommand = {
  copyLabel: string
  label: string
  value: string
}

export function cloneCommands({
  actor,
  owner,
  readerView,
  remoteUrl,
  repo,
  view,
}: {
  actor: RepositoryActor
  owner: string
  readerView: ViewId
  remoteUrl: string
  repo: string
  view: ViewId
}): CloneCommand[] {
  if (actor === 'Public') {
    return [{ copyLabel: 'Copy Git clone command', label: 'Git over HTTPS', value: `git clone ${remoteUrl}` }]
  }
  const viewFlag = view === readerView ? '' : ` --view ${view}`
  return [
    { copyLabel: 'Copy Scope CLI clone command', label: 'Scope CLI', value: `scope clone ${owner}/${repo}${viewFlag}` },
    { copyLabel: 'Copy Git remote URL', label: 'Git remote', value: remoteUrl },
  ]
}
