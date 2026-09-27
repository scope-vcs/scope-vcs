import { File, Folder, GitBranch, LockKeyhole } from 'lucide-react'
import type { CSSProperties, ReactElement } from 'react'
import { repoPanel } from './landing-copy'
import { Note, Swap, useLandingView } from './landing-view'

const icon = 'size-4 shrink-0 stroke-[1.6] text-muted-foreground'
const row = 'landing-rise flex h-[38px] items-center gap-3 rounded-md px-2.5 -mx-2.5'
const pathCount = {
  public: `${repoPanel.shared.length} paths`,
  private: `${repoPanel.shared.length + repoPanel.kept.length} paths`,
}

/** The repository's top level. Shared paths show in both views; kept paths are
 * laid out in both but only visible through the lens. */
export function RepoPanel(): ReactElement {
  const view = useLandingView()
  return (
    <div className="repo-panel relative min-w-0 font-mono text-sm" aria-label={view === 'public' ? `Public clone of ${repoPanel.name}` : undefined} role={view === 'public' ? 'figure' : undefined}>
      <div className="landing-rise flex h-11 items-center gap-2.5 border-b border-foreground text-[13px] [--rise-delay:300ms]">
        <GitBranch aria-hidden className={icon} />
        {repoPanel.name}
        <span className="ml-auto text-xs text-muted-foreground tabular-nums"><Swap text={pathCount} /></span>
      </div>
      <ul className="py-2">
        {repoPanel.shared.map((path, index) => (
          <li className={`${row} hover:bg-muted`} key={path.name} style={{ '--rise-delay': `${360 + index * 55}ms` } as CSSProperties}>
            {path.name.endsWith('/') ? <Folder aria-hidden className={icon} /> : <File aria-hidden className={icon} />}
            {path.name}
            <span className="ml-auto grid min-w-7 place-items-center justify-items-end text-xs text-muted-foreground *:[grid-area:1/1]">
              <span className="landing-public-only">{path.age}</span>
              <span className="landing-private-only size-1.5 rounded-full bg-success" />
            </span>
          </li>
        ))}
        {repoPanel.kept.map((name, index) => (
          <li className={`${row} landing-private-only is-kept`} data-lens-anchor={index === 0 || undefined} key={name} style={{ '--rise-delay': `${1250 + index * 90}ms` } as CSSProperties}>
            {name.endsWith('/') ? <Folder aria-hidden className={icon} /> : <File aria-hidden className={icon} />}
            {name}
            <LockKeyhole aria-hidden className="ml-auto size-3.5 stroke-[1.6] text-foreground" />
          </li>
        ))}
      </ul>
      <Note className="absolute left-0 top-[calc(100%+20px)] max-w-[30ch]" id="repo" />
    </div>
  )
}
