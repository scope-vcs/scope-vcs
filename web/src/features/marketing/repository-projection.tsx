import type { Visibility, VisibilityState } from '@/api/types'
import {
  buildFileSystemTree,
  folderVisibility,
  type FileSystemTreeNode,
} from '@/components/file-system-tree-model'
import { VisibilityBadge, VisibilityLegend } from '@/components/visibility-badge'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderGit2,
  FolderOpen,
} from 'lucide-react'
import { useState, type ReactElement } from 'react'

type ProjectionAudience = 'private' | 'public'

type ProjectionFile = {
  path: string
  visibility: Visibility
}

type ProjectionRow = {
  depth: number
  expanded?: boolean
  key: string
  name: string
  path: string
  type: 'file' | 'folder'
  visibility: VisibilityState
}

type ProjectionViewDefinition = {
  audience: ProjectionAudience
  label: string
  rows: readonly ProjectionRow[]
}

type ProjectionViewProps = ProjectionViewDefinition & {
  hoveredPath: string | null
  onHoverRow: (row: ProjectionRow | null) => void
}

const repositoryFiles: readonly ProjectionFile[] = [
  { path: 'src/cli/index.ts', visibility: 'Public' },
  { path: 'src/internal/policy.ts', visibility: 'Private' },
  { path: 'src/shared/config.ts', visibility: 'Public' },
  { path: '.env', visibility: 'Private' },
  { path: 'README.md', visibility: 'Public' },
]

const expandedFolderPaths = new Set(['/src', '/src/cli'])

const treeRowMetrics = {
  chevronSize: 12,
  disclosureSlotSize: 16,
  fileIconSize: 16,
  itemGap: 4,
  levelIndent: 10,
  rowInset: 4,
} as const
const fileIconInset = (
  treeRowMetrics.disclosureSlotSize - treeRowMetrics.chevronSize
) / 2
const fileLabelGap = treeRowMetrics.disclosureSlotSize
  + treeRowMetrics.itemGap
  - fileIconInset
  - treeRowMetrics.fileIconSize

const projectionViews = [
  {
    audience: 'public',
    label: 'public view',
    rows: buildProjectionRows(
      repositoryFiles.filter((file) => file.visibility === 'Public'),
    ),
  },
  {
    audience: 'private',
    label: 'maintainer view',
    rows: buildProjectionRows(repositoryFiles),
  },
] as const satisfies ReadonlyArray<ProjectionViewDefinition>

export function RepositoryProjection(): ReactElement {
  const [hoveredRow, setHoveredRow] = useState<ProjectionRow | null>(null)
  const sourceContext = hoveredRow ? projectionSourcePath(hoveredRow) : null

  return (
    <section
      aria-labelledby="repository-views-title"
      className="marketing-projection"
      id="repository-views"
    >
      <h2 className="sr-only" id="repository-views-title">
        One repository with public and maintainer views
      </h2>

      <div
        className="marketing-source-node"
        data-projection-node="repository"
        id="repository-source"
      >
        <span aria-hidden className="marketing-source-icon">
          <FolderGit2 />
        </span>
        <span className="marketing-source-copy">
          <strong>scope/</strong>
          {sourceContext && <span title={sourceContext}>{sourceContext}</span>}
        </span>
        <span className="marketing-source-branch">main</span>
      </div>

      <div aria-hidden className="marketing-projection-arrows">
        <span>↓</span>
        <span>↓</span>
      </div>
      <div className="marketing-views">
        {projectionViews.map((view) => (
          <ProjectionView
            hoveredPath={hoveredRow?.path ?? null}
            key={view.audience}
            onHoverRow={setHoveredRow}
            {...view}
          />
        ))}
      </div>
      <div className="mt-4">
        <VisibilityLegend />
      </div>
      <p className="mt-3 text-sm leading-relaxed text-muted-foreground" id="projection-explanation">
        The public receives only shared files. Maintainers work with the complete repository.
      </p>
    </section>
  )
}

function ProjectionView({
  audience,
  hoveredPath,
  label,
  onHoverRow,
  rows,
}: ProjectionViewProps): ReactElement {
  return (
    <article
      className={`marketing-view marketing-view-${audience}`}
      data-projection-node={audience}
    >
      <header className="marketing-view-header">
        <h3>{label}</h3>
      </header>
      <ul className="p-1 sm:p-2">
        {rows.map((row) => (
          <li key={row.key}>
            <button
              aria-label={`${row.path}, ${row.visibility.toLowerCase()}`}
              aria-describedby="projection-explanation"
              className="marketing-file-row w-full text-left"
              type="button"
              onFocus={() => onHoverRow(row)}
              onBlur={() => onHoverRow(null)}
              data-highlighted={hoveredPath === row.path || undefined}
              data-path={row.path}
              onPointerEnter={() => onHoverRow(row)}
              onPointerLeave={() => onHoverRow(null)}
              style={{ paddingLeft: projectionRowInset(row) }}
            >
              <span
                className="flex min-w-0 items-center"
                style={{ gap: row.type === 'file' ? fileLabelGap : treeRowMetrics.itemGap }}
              >
                {row.type === 'folder' && (
                  <span
                    aria-hidden
                    className="marketing-disclosure grid size-4 shrink-0 place-items-center text-[var(--platinum)]"
                  >
                    <ProjectionDisclosureIcon expanded={row.expanded} />
                  </span>
                )}
                <ProjectionFileIcon expanded={row.expanded} type={row.type} />
                <span className="min-w-0 truncate font-mono text-xs">{row.name}</span>
              </span>
              <VisibilityBadge compact visibility={row.visibility} />
            </button>
          </li>
        ))}
      </ul>
    </article>
  )
}

function buildProjectionRows(
  files: readonly ProjectionFile[],
): ProjectionRow[] {
  const tree = buildFileSystemTree([...files])
  return flattenProjectionTree(tree.children)
}

function flattenProjectionTree(
  nodes: FileSystemTreeNode<ProjectionFile>[],
  depth = 0,
): ProjectionRow[] {
  return nodes.flatMap((node) => {
    if (node.type === 'file') {
      return [{
        depth,
        key: node.key,
        name: node.name,
        path: node.path,
        type: node.type,
        visibility: node.file.visibility,
      }]
    }

    const expanded = expandedFolderPaths.has(node.path)
    const row: ProjectionRow = {
      depth,
      expanded,
      key: node.key,
      name: `${node.name}/`,
      path: node.path,
      type: node.type,
      visibility: folderVisibility(node.files),
    }

    return expanded
      ? [row, ...flattenProjectionTree(node.children, depth + 1)]
      : [row]
  })
}

function projectionRowInset(row: ProjectionRow): number {
  const depthInset = treeRowMetrics.rowInset
    + row.depth * treeRowMetrics.levelIndent

  // A file icon replaces the disclosure slot: its edge aligns with a
  // centered chevron, while the derived gap keeps labels aligned with folders.
  return row.type === 'file' ? depthInset + fileIconInset : depthInset
}

function projectionSourcePath(row: ProjectionRow): string {
  const path = row.path.replace(/^\//, '')
  return row.type === 'folder' ? `${path}/` : path
}

function ProjectionDisclosureIcon({
  expanded,
}: Pick<ProjectionRow, 'expanded'>): ReactElement {
  if (expanded) return <ChevronDown className="size-3" />
  return <ChevronRight className="size-3" />
}

function ProjectionFileIcon({
  expanded,
  type,
}: Pick<ProjectionRow, 'expanded' | 'type'>): ReactElement {
  const className = 'size-4 shrink-0 text-[var(--platinum)]'

  if (type === 'file') {
    return <File aria-hidden className={className} strokeWidth={1.7} />
  }

  if (expanded) {
    return <FolderOpen aria-hidden className={className} strokeWidth={1.7} />
  }

  return <Folder aria-hidden className={className} strokeWidth={1.7} />
}
