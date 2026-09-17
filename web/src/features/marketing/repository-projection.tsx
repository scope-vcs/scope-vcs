import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'
import { cn } from '@/lib/utils'

const files = ['sdk/', 'docs/', 'examples/', 'internal/', 'README.md']

function VisibilityLabel({ isPublic }: { isPublic: boolean }): ReactElement {
  const Icon = isPublic ? Globe : LockKeyhole
  return (
    <span className={`ml-auto inline-flex items-center justify-end text-(length:--marketing-figure-meta) ${isPublic ? 'is-public text-success-strong' : 'is-private text-muted-foreground'}`}>
      <Icon aria-hidden className="icon hidden size-3.5 max-[521px]:block min-[901px]:max-[1151px]:block max-[361px]:size-3" />
      <span className="max-[521px]:hidden min-[901px]:max-[1151px]:hidden">{isPublic ? 'Public' : 'Private'}</span>
    </span>
  )
}

function RepositoryColumn({ publicClone = false }: { publicClone?: boolean }): ReactElement {
  const TitleIcon = publicClone ? Globe : LockKeyhole
  return (
    <div className={cn('min-w-0', publicClone && 'border-l border-border bg-muted')}>
      <div className="flex h-11 items-center gap-[9px] border-b border-border px-5 text-(length:--marketing-figure-title) font-medium max-[1151px]:gap-[7px] max-[1151px]:px-4 max-[521px]:gap-1.5 max-[521px]:px-[11px] max-[361px]:gap-1 max-[361px]:px-[7px]">
        <TitleIcon aria-hidden className={cn('icon size-4 max-[521px]:size-3.5 max-[361px]:size-3', publicClone ? 'text-success-strong' : 'text-muted-foreground')} />
        {publicClone ? 'Public clone' : 'Your repository'}
      </div>
      <ul className="m-0 list-none px-3.5 py-3 max-[1151px]:px-2.5 max-[521px]:px-1.5 max-[521px]:py-2 max-[361px]:px-[3px]">
        {files.map((name) => {
          const isExample = name === 'examples/'
          const absent = publicClone && name === 'internal/'
          const Icon = name.endsWith('/') ? Folder : File
          return (
            <li
              key={name}
              aria-hidden={absent || undefined}
              className={cn(
                'repository-file relative flex h-[38px] min-w-0 items-center gap-[9px] rounded-sm px-1.5 font-mono text-(length:--marketing-figure-body) [line-height:normal] max-[1151px]:gap-[7px] max-[521px]:gap-1.5 max-[521px]:px-[5px] max-[361px]:gap-1 max-[361px]:px-1',
                isExample && (publicClone ? 'shared-example' : 'source-example'),
              )}
            >
              {!absent && <>
                <Icon aria-hidden className="icon size-4 text-muted-foreground max-[521px]:size-3.5 max-[361px]:size-3" />
                <span className="whitespace-nowrap">{name}</span>
                {publicClone ? (
                  <Check aria-hidden className="icon ml-auto size-3.5 text-success-strong max-[521px]:size-3 max-[361px]:size-[11px]" />
                ) : isExample ? (
                  <span className="visibility-changing relative ml-auto h-[18px] w-14 shrink-0 *:absolute *:inset-0 max-[521px]:w-4 min-[901px]:max-[1151px]:w-4 max-[361px]:w-3">
                    <VisibilityLabel isPublic={false} />
                    <VisibilityLabel isPublic />
                  </span>
                ) : <VisibilityLabel isPublic={name !== 'internal/'} />}
              </>}
            </li>
          )
        })}
      </ul>
    </div>
  )
}

export function RepositoryProjection(): ReactElement {
  return (
    <div className="min-w-0 w-full">
      <figure
        aria-label="One repository with public and private files. The public clone contains shared folders. The examples folder is shared and then made private in a repeating illustration; internal code stays private."
        className="repository overflow-hidden rounded-md border border-border bg-card [--marketing-cycle:8s]"
      >
        <div aria-hidden className="flex h-11 items-center justify-between gap-4 border-b border-border px-5 font-mono text-(length:--marketing-figure-title) font-medium [line-height:normal] max-[1151px]:px-4 max-[521px]:px-3.5">
          <span className="flex items-center gap-2.5">
            <GitBranch aria-hidden className="icon size-4 text-muted-foreground" />
            acme / toolkit
          </span>
          <span className="text-(length:--marketing-figure-meta) text-muted-foreground">main</span>
        </div>
        <div aria-hidden className="grid grid-cols-2">
          <RepositoryColumn />
          <RepositoryColumn publicClone />
        </div>
      </figure>
    </div>
  )
}
