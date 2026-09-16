import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'
import { cn } from '@/lib/utils'

function ContributionTree({ merged = false }: { merged?: boolean }): ReactElement {
  const files = merged ? ['sdk/api.ts', 'docs/', 'internal/', 'README.md'] : ['sdk/api.ts', 'docs/', 'README.md']
  return (
    <ul className="m-0 list-none p-0">
      {files.map((name) => {
        const changed = name === 'sdk/api.ts'
        const privateFile = name === 'internal/'
        const Icon = name.endsWith('/') ? Folder : File
        const StateIcon = privateFile ? LockKeyhole : Check
        return (
          <li key={name} className={cn('scene-file relative flex h-8 items-center gap-[9px] rounded-[3px] font-mono text-[13px] [line-height:normal]', changed && (merged ? 'merged-file' : 'submitted-file'))}>
            <Icon className="icon size-[15px] text-muted-foreground" />
            {name}
            {(changed || privateFile) && (
              <span className={cn('ml-auto flex items-center gap-1.5 text-xs', privateFile ? 'text-muted-foreground' : 'text-success-strong')}>
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
      className="contribution-flow min-w-0 [--marketing-request-cycle:14s]"
    >
      <div aria-hidden className="request-progress mb-4 grid grid-cols-3 gap-3.5 max-[521px]:mb-4 max-[521px]:gap-2.5">
        {['Submitted', 'Your review', 'Merged'].map((step) => (
          <div key={step} className="request-step relative border-b border-border pb-2 text-[13px] font-medium text-muted-foreground">{step}</div>
        ))}
      </div>
      <div aria-hidden className="request-sheet relative border-y border-border bg-card [--marketing-request-inset:18px] max-[521px]:[--marketing-request-inset:12px]">
        <div className="flex min-h-[52px] items-center gap-3 border-b border-border py-3 px-[var(--marketing-request-inset)] max-[521px]:min-h-[56px] max-[521px]:gap-2">
          <GitBranch className="icon size-[18px] text-success-strong max-[521px]:hidden" />
          <h3 className="m-0 min-w-0 font-sans text-[13px] font-medium leading-[1.35] tracking-normal max-[521px]:max-w-[195px] max-[361px]:max-w-[175px]">Handle empty responses</h3>
          <div className="relative ml-auto h-5 w-[9ch] shrink-0 text-right font-mono text-xs leading-5 whitespace-nowrap text-muted-foreground *:absolute *:inset-0">
            <span className="state-submitted">Open</span>
            <span className="state-review text-success-strong">In review</span>
            <span className="state-merged text-success-strong">Merged</span>
          </div>
        </div>
        <div className="relative h-[240px] max-[521px]:h-[260px] overflow-hidden *:pointer-events-none *:absolute *:inset-0 *:pt-4 *:px-[var(--marketing-request-inset)] *:pb-4 *:will-change-[opacity,transform] max-[521px]:*:pt-4">
          <div className="submission-scene">
            <div className="mb-[15px] flex items-center gap-2 text-[13px] font-medium leading-5">
              <Globe className="icon size-[15px] text-muted-foreground" />
              Public clone
            </div>
            <ContributionTree />
            <div className="absolute inset-x-[var(--marketing-request-inset)] bottom-[21px] flex items-center gap-2 text-xs text-muted-foreground">
              <Check className="icon size-3.5 text-success-strong" />
              Submitted to your review queue
            </div>
          </div>
          <div className="review-scene">
            <div className="mb-[13px] flex items-center justify-between gap-4 font-mono text-[13px] [line-height:normal]">
              <span className="flex items-center gap-2">
                <File className="icon size-3.5 text-muted-foreground" />
                sdk/api.ts
              </span>
              <span className="text-xs text-success-strong">+1 line</span>
            </div>
            <div className="review-code m-0 font-mono text-[13px] leading-[1.9] text-muted-foreground *:block *:px-2 *:whitespace-pre-wrap *:[overflow-wrap:anywhere] max-[521px]:leading-[1.8] max-[361px]:*:px-[5px]">
              <span>  const res = await fetch(url);</span>
              <span className="added-line relative overflow-hidden bg-success-soft text-success-strong">
                + if (res.status === 204) return null;
              </span>
              <span>  return res.json();</span>
            </div>
            <div className="maintainer-review mt-4 grid grid-cols-[26px_minmax(0,1fr)] gap-2.5 max-[521px]:mt-4">
              <span className="grid size-[26px] place-items-center rounded-full border border-border bg-muted font-mono text-[11px] text-muted-foreground [line-height:normal]">M</span>
              <div>
                <div className="text-xs leading-[1.4] font-medium">Maintainer</div>
                <p className="mt-1 text-[13px] leading-normal text-muted-foreground">
                  Empty responses return null. Looks good.
                </p>
              </div>
            </div>
            <div className="review-decision absolute bottom-5 left-[calc(var(--marketing-request-inset)+36px)] flex items-center gap-[7px] text-xs text-success-strong max-[521px]:bottom-[18px]">
              <Check className="icon size-3.5" />
              Ready to merge
            </div>
          </div>
          <div className="merged-scene">
            <div className="mb-[15px] flex items-center gap-2 text-[13px] font-medium leading-5">
              <GitBranch className="icon size-[15px] text-muted-foreground" />
              Your repository
              <span className="ml-auto font-mono text-xs font-normal text-muted-foreground [line-height:normal]">main</span>
            </div>
            <ContributionTree merged />
            <div className="absolute inset-x-[var(--marketing-request-inset)] bottom-[21px] flex items-center gap-2 text-xs text-muted-foreground">
              <Check className="icon size-3.5 text-success-strong" />
              Merged by the maintainer
            </div>
          </div>
        </div>
      </div>
    </figure>
  )
}
