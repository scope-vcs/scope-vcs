import { Check, File, Folder, GitBranch, Globe, LockKeyhole } from 'lucide-react'
import type { ReactElement } from 'react'
import { cn } from '@/lib/utils'

const files = ['sdk/', 'docs/', 'examples/', 'internal/', 'README.md']

function VisibilityLabel({ isPublic }: { isPublic: boolean }): ReactElement {
  const Icon = isPublic ? Globe : LockKeyhole
  return (
    <span className={`visibility-label ml-auto inline-flex items-center justify-end text-landing-meta ${isPublic ? 'is-public text-landing-green' : 'is-private text-landing-muted'}`}>
      <Icon aria-hidden className="icon hidden size-3.5 max-[521px]:block min-[901px]:max-[1151px]:block max-[361px]:size-3" />
      <span className="label-text max-[521px]:hidden min-[901px]:max-[1151px]:hidden">{isPublic ? 'Public' : 'Private'}</span>
    </span>
  )
}

function RepositoryColumn({ publicClone = false }: { publicClone?: boolean }): ReactElement {
  const TitleIcon = publicClone ? Globe : LockKeyhole
  return (
    <div className={cn('repository-column min-w-0', publicClone && 'public-column border-l border-landing-line bg-landing-panel')}>
      <div className="repository-title flex h-15 items-center gap-[9px] border-b border-landing-line px-5 text-landing-title font-medium max-[1151px]:gap-[7px] max-[1151px]:px-4 max-[521px]:h-13 max-[521px]:gap-1.5 max-[521px]:px-[11px] max-[361px]:gap-1 max-[361px]:px-[7px]">
        <TitleIcon aria-hidden className={cn('icon size-4 max-[521px]:size-3.5 max-[361px]:size-3', publicClone ? 'text-landing-green' : 'text-landing-muted')} />
        {publicClone ? 'Public clone' : 'Your repository'}
      </div>
      <ul className="repository-files m-0 list-none px-3.5 pt-[13px] pb-5 max-[1151px]:px-2.5 max-[521px]:px-1.5 max-[521px]:pt-2.5 max-[521px]:pb-[15px] max-[361px]:px-[3px]">
        {files.map((name) => {
          const isExample = name === 'examples/'
          const absent = publicClone && name === 'internal/'
          const Icon = name.endsWith('/') ? Folder : File
          return (
            <li
              key={name}
              aria-hidden={absent || undefined}
              className={cn(
                'repository-file relative flex h-[50px] min-w-0 items-center gap-[9px] rounded-sm px-1.5 font-mono text-landing-body [line-height:normal] max-[1151px]:gap-[7px] max-[521px]:h-[46px] max-[521px]:gap-1.5 max-[521px]:px-[5px] max-[361px]:gap-1 max-[361px]:px-1',
                absent && 'absent',
                isExample && (publicClone ? 'shared-example' : 'source-example'),
              )}
            >
              {!absent && <>
                <Icon aria-hidden className="icon size-4 text-landing-muted max-[521px]:size-3.5 max-[361px]:size-3" />
                <span className="repository-filename whitespace-nowrap">{name}</span>
                {publicClone ? (
                  <Check aria-hidden className="icon ml-auto size-3.5 text-landing-green max-[521px]:size-3 max-[361px]:size-[11px]" />
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
    <div className="demo min-w-0 w-full self-center">
      <figure
        aria-label="One repository with public and private files. The public clone contains shared folders. The examples folder is shared and then made private in a repeating illustration; internal code stays private."
        className="repository overflow-hidden rounded-md border border-landing-line bg-landing-paper [--landing-cycle:8s]"
      >
        <div aria-hidden className="repository-header flex h-[58px] items-center justify-between gap-4 border-b border-landing-line px-5 font-mono text-landing-title [line-height:normal] max-[1151px]:px-4 max-[521px]:h-[50px] max-[521px]:px-3.5">
          <span className="repository-name flex items-center gap-2.5">
            <GitBranch aria-hidden className="icon size-4 text-landing-muted" />
            acme / toolkit
          </span>
          <span className="repository-branch text-landing-meta text-landing-muted">main</span>
        </div>
        <div aria-hidden className="repository-views grid grid-cols-2">
          <RepositoryColumn />
          <RepositoryColumn publicClone />
        </div>
      </figure>
    </div>
  )
}
