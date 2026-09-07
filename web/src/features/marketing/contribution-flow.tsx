import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'
import { cn } from '@/lib/utils'

function ContributionTree({ merged = false }: { merged?: boolean }): ReactElement {
  const files = merged ? ['sdk/api.ts', 'docs/', 'internal/', 'README.md'] : ['sdk/api.ts', 'docs/', 'README.md']
  return (
    <ul className="scene-tree m-0 list-none p-0">
      {files.map((name) => {
        const changed = name === 'sdk/api.ts'
        const privateFile = name === 'internal/'
        const Icon = name.endsWith('/') ? Folder : File
        const StateIcon = privateFile ? LockKeyhole : Check
        return (
          <li key={name} className={cn('scene-file relative flex h-10 items-center gap-[9px] rounded-[3px] font-mono text-landing-body [line-height:normal]', changed && (merged ? 'merged-file' : 'submitted-file'))}>
            <Icon className="icon size-[15px] text-landing-muted" />
            {name}
            {(changed || privateFile) && (
              <span className={`file-state ml-auto flex items-center gap-1.5 text-landing-meta ${privateFile ? 'private-state text-landing-muted' : 'text-landing-green'}`}>
                {merged && <StateIcon className="icon size-3" />}
                {privateFile ? 'Private' : merged ? 'Updated' : '+1'}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

export function ContributionFlow(): ReactElement {
  return (
    <figure
      aria-label="A request to handle empty API responses moves from a public clone to maintainer review. The maintainer reads the one-line change, comments that it looks good, then merges it into the full repository. Internal code remains private."
      className="contribution-flow min-w-0 [--landing-request-cycle:14s]"
    >
      <div aria-hidden className="request-progress mb-6 grid grid-cols-3 gap-3.5 max-[521px]:mb-[19px] max-[521px]:gap-2.5">
        {['Submitted', 'Your review', 'Merged'].map((step) => (
          <div key={step} className="request-step relative border-b border-landing-line pb-3 text-landing-title text-landing-muted">{step}</div>
        ))}
      </div>
      <div aria-hidden className="request-sheet relative border-y border-landing-line bg-landing-paper [--landing-request-inset:18px] max-[521px]:[--landing-request-inset:12px]">
        <div className="request-heading flex min-h-[65px] items-center gap-3 border-b border-landing-line py-4 pr-[var(--landing-request-inset)] max-[521px]:min-h-[70px] max-[521px]:gap-2">
          <GitBranch className="icon size-[18px] text-landing-green max-[521px]:hidden" />
          <h3 className="m-0 min-w-0 font-sans text-landing-title leading-[1.35] font-medium tracking-normal max-[521px]:max-w-[195px] max-[361px]:max-w-[175px]">Handle empty responses</h3>
          <div className="request-state-stack relative ml-auto h-5 w-[9ch] shrink-0 text-right font-mono text-landing-meta leading-5 whitespace-nowrap text-landing-muted *:absolute *:inset-0">
            <span className="state-submitted">Open</span>
            <span className="state-review text-landing-green">In review</span>
            <span className="state-merged text-landing-green">Merged</span>
          </div>
        </div>
        <div className="request-scenes relative h-[280px] overflow-hidden *:pointer-events-none *:absolute *:inset-0 *:pt-[22px] *:pr-[var(--landing-request-inset)] *:pb-[18px] *:will-change-[opacity,transform] max-[521px]:*:pt-[19px]">
          <div className="request-scene submission-scene">
            <div className="scene-context mb-[15px] flex items-center gap-2 text-landing-title leading-5 font-medium">
              <Globe className="icon size-[15px] text-landing-muted" />
              Public clone
            </div>
            <ContributionTree />
            <div className="scene-receipt absolute inset-x-0 bottom-[21px] flex items-center gap-2 text-landing-meta text-landing-muted">
              <Check className="icon size-3.5 text-landing-green" />
              Submitted to your review queue
            </div>
          </div>
          <div className="request-scene review-scene">
            <div className="review-file-header mb-[13px] flex items-center justify-between gap-4 font-mono text-landing-body [line-height:normal]">
              <span className="flex items-center gap-2">
                <File className="icon size-3.5 text-landing-muted" />
                sdk/api.ts
              </span>
              <span className="text-landing-meta text-landing-green">+1 line</span>
            </div>
            <div className="review-code m-0 font-mono text-landing-body leading-[1.9] text-landing-muted *:block *:px-2 *:whitespace-pre-wrap *:[overflow-wrap:anywhere] max-[521px]:leading-[1.8] max-[361px]:*:px-[5px]">
              <span>  const response = await fetch(url);</span>
              <span className="added-line relative overflow-hidden bg-landing-green-soft text-landing-green">
                + if (response.status === 204) return null;
              </span>
              <span>  return response.json();</span>
            </div>
            <div className="maintainer-review mt-[19px] grid grid-cols-[26px_minmax(0,1fr)] gap-2.5 max-[521px]:mt-4">
              <span className="reviewer-mark grid size-[26px] place-items-center rounded-full border border-landing-line bg-landing-panel font-mono text-[11px] text-landing-muted [line-height:normal]">M</span>
              <div>
                <div className="reviewer-name text-landing-meta leading-[1.4] font-medium">Maintainer</div>
                <p className="review-comment-text mt-1 text-landing-body leading-normal text-landing-muted">
                  Empty responses return null. Looks good.
                </p>
              </div>
            </div>
            <div className="review-decision absolute bottom-5 left-9 flex items-center gap-[7px] text-landing-meta text-landing-green max-[521px]:bottom-[18px]">
              <Check className="icon size-3.5" />
              Ready to merge
            </div>
          </div>
          <div className="request-scene merged-scene">
            <div className="scene-context mb-[15px] flex items-center gap-2 text-landing-title leading-5 font-medium">
              <GitBranch className="icon size-[15px] text-landing-muted" />
              Your repository
              <span className="scene-branch ml-auto font-mono text-landing-meta font-normal text-landing-muted [line-height:normal]">main</span>
            </div>
            <ContributionTree merged />
            <div className="scene-receipt absolute inset-x-0 bottom-[21px] flex items-center gap-2 text-landing-meta text-landing-muted">
              <Check className="icon size-3.5 text-landing-green" />
              Merged by the maintainer
            </div>
          </div>
        </div>
      </div>
    </figure>
  )
}
