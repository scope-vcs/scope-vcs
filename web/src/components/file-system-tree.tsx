import type { ReactNode } from 'react'
import { useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton, type TextSkeletonLength } from '@/components/ui/text-skeleton'
import { VisibilityBadge, VisibilityLegend } from '@/components/visibility-badge'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
} from 'lucide-react'
import {
  buildFileSystemTree,
  ancestorFolderKeys,
  normalizeFilePath,
  folderVisibility,
  type FileSystemTreeFileBase,
  type FileSystemTreeNode,
} from './file-system-tree-model'

const VISIBLE_TREE_COLUMNS = 'grid-cols-[minmax(0,1fr)_auto_20px]'
const HIDDEN_VISIBILITY_TREE_COLUMNS = 'grid-cols-[minmax(0,1fr)_auto]'

function treeColumnsClassName(showVisibility: boolean) {
  return showVisibility ? VISIBLE_TREE_COLUMNS : HIDDEN_VISIBILITY_TREE_COLUMNS
}

const TREE_HEADER_CLASS =
  'hidden gap-3 px-3 pb-1.5 pt-1 text-xs font-medium text-muted-foreground sm:grid sm:items-center'
const TREE_ROW_CLASS =
  'grid min-h-9 items-center gap-2 rounded-md border border-transparent px-3 py-1.5 text-sm'
const PENDING_TREE_ROWS: TextSkeletonLength[] = ['medium', 'long', 'short', 'long', 'medium', 'long']

export function FileSystemTreeSkeleton({
  metaColumnLabel,
  showVisibility = true,
}: {
  metaColumnLabel: ReactNode
  showVisibility?: boolean
}) {
  const columnsClassName = treeColumnsClassName(showVisibility)
  return (
    <div>
      {metaColumnLabel !== null && (
        <div className={cn(TREE_HEADER_CLASS, columnsClassName)}>
          <div>Path</div>
          <div>{metaColumnLabel}</div>
          {showVisibility && <div />}
        </div>
      )}
      <ul className="space-y-0.5">
        {PENDING_TREE_ROWS.map((length, row) => (
          <li className={cn(TREE_ROW_CLASS, columnsClassName)} key={`file-${row}`}>
            <div className="flex min-w-0 items-center gap-2">
              <BlockSkeleton className="size-4 shrink-0" />
              <TextSkeleton length={length} size="meta" />
            </div>
            <div />
            {showVisibility && <BlockSkeleton className="size-3.5 rounded-full" />}
          </li>
        ))}
      </ul>
      {showVisibility && (
        <div className="px-3 pt-4 pb-2"><TextSkeleton length="medium" size="meta" /></div>
      )}
    </div>
  )
}

export function FileSystemTree<TFile extends FileSystemTreeFileBase>({
  expandFolders = false,
  files,
  getFileMeta,
  metaColumnLabel = null,
  onActivateFile,
  onSelectFile,
  selectedFilePath = null,
  showVisibility,
}: {
  expandFolders?: boolean
  files: TFile[]
  getFileMeta?: (file: TFile) => ReactNode
  metaColumnLabel?: ReactNode
  onActivateFile?: (file: TFile) => void
  onSelectFile?: (file: TFile) => void
  selectedFilePath?: string | null
  showVisibility: boolean
}) {
  const root = useMemo(() => buildFileSystemTree(files), [files])
  const treeKey = useMemo(
    () => files.map((file) => normalizeFilePath(file.path)).join('\0'),
    [files],
  )
  const columnsClassName = treeColumnsClassName(showVisibility)

  return (
    <FileSystemTreeRows
      columnsClassName={columnsClassName}
      expandFolders={expandFolders}
      getFileMeta={getFileMeta}
      key={treeKey}
      onActivateFile={onActivateFile}
      onSelectFile={onSelectFile}
      root={root}
      selectedFilePath={selectedFilePath}
      showVisibility={showVisibility}
      metaColumnLabel={metaColumnLabel}
    />
  )
}

function FileSystemTreeRows<TFile extends FileSystemTreeFileBase>({
  columnsClassName,
  expandFolders,
  getFileMeta,
  metaColumnLabel,
  onActivateFile,
  onSelectFile,
  root,
  selectedFilePath,
  showVisibility,
}: {
  columnsClassName: string
  expandFolders: boolean
  getFileMeta?: (file: TFile) => ReactNode
  metaColumnLabel: ReactNode
  onActivateFile?: (file: TFile) => void
  onSelectFile?: (file: TFile) => void
  root: Extract<FileSystemTreeNode<TFile>, { type: 'folder' }>
  selectedFilePath: string | null
  showVisibility: boolean
}) {
  const [folderState, setFolderState] = useState<FolderState>(() => ({
    collapsedForSelection: new Map(),
    expanded: new Set(expandFolders ? folderKeys(root) : []),
  }))
  const selectedPath = normalizeFilePath(selectedFilePath ?? '')
  const selectedAncestorKeys = useMemo(
    () => new Set(ancestorFolderKeys(selectedPath)),
    [selectedPath],
  )

  function toggleFolder(key: string) {
    setFolderState((current) => {
      const next: FolderState = {
        collapsedForSelection: new Map(current.collapsedForSelection),
        expanded: new Set(current.expanded),
      }
      if (folderIsCollapsed(current, key, selectedAncestorKeys, selectedPath)) {
        next.expanded.add(key)
        next.collapsedForSelection.delete(key)
      } else {
        next.expanded.delete(key)
        if (selectedAncestorKeys.has(key)) {
          next.collapsedForSelection.set(key, selectedPath)
        }
      }
      return next
    })
  }

  return (
    <div>
      {metaColumnLabel !== null && (
        <div className={cn(TREE_HEADER_CLASS, columnsClassName)}>
          <div>Path</div>
          <div>{metaColumnLabel}</div>
          {showVisibility && (
            <div className="text-center">
              <span className="sr-only">Visibility</span>
            </div>
          )}
        </div>
      )}
      <ul className="space-y-0.5">
        {root.children.map((node) => (
          <FileSystemTreeNodeRow
            folderState={folderState}
            columnsClassName={columnsClassName}
            depth={0}
            getFileMeta={getFileMeta}
            key={node.key}
            node={node}
            onActivateFile={onActivateFile}
            onSelectFile={onSelectFile}
            onToggleFolder={toggleFolder}
            selectedAncestorKeys={selectedAncestorKeys}
            selectedFilePath={selectedFilePath}
            showVisibility={showVisibility}
          />
        ))}
      </ul>
      {showVisibility ? <div className="px-3 pt-4 pb-2"><VisibilityLegend /></div> : null}
    </div>
  )
}

function FileSystemTreeNodeRow<TFile extends FileSystemTreeFileBase>({
  folderState,
  columnsClassName,
  depth,
  getFileMeta,
  node,
  onActivateFile,
  onSelectFile,
  onToggleFolder,
  selectedAncestorKeys,
  selectedFilePath,
  showVisibility,
}: {
  folderState: FolderState
  columnsClassName: string
  depth: number
  getFileMeta?: (file: TFile) => ReactNode
  node: FileSystemTreeNode<TFile>
  onActivateFile?: (file: TFile) => void
  onSelectFile?: (file: TFile) => void
  onToggleFolder: (key: string) => void
  selectedAncestorKeys: ReadonlySet<string>
  selectedFilePath: string | null
  showVisibility: boolean
}) {
  if (node.type === 'file') {
    const selected =
      selectedFilePath !== null &&
      normalizeFilePath(selectedFilePath) === normalizeFilePath(node.file.path)
    return (
      <li
        className={cn(
          'relative transition-[background-color,border-color] hover:bg-accent/50',
          TREE_ROW_CLASS,
          selected &&
            'border-[var(--border-strong)] bg-muted shadow-[inset_2px_0_0_0_var(--foreground)] hover:bg-muted',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          {onSelectFile ? (
            <button
              aria-current={selected ? 'true' : undefined}
              className="flex min-w-0 flex-1 items-center gap-2 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
              onClick={() => onSelectFile(node.file)}
              onDoubleClick={
                onActivateFile ? () => onActivateFile(node.file) : undefined
              }
              type="button"
            >
              <FilePathLabel name={node.name} path={node.path} />
            </button>
          ) : (
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <FilePathLabel name={node.name} path={node.path} />
            </div>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-xs leading-4">
          {getFileMeta?.(node.file)}
        </div>
        {showVisibility && (
          <div className="flex items-center justify-end gap-1.5 sm:justify-center">
            <VisibilityBadge compact visibility={node.file.label} />
          </div>
        )}
      </li>
    )
  }

  const isCollapsed = folderIsCollapsed(
    folderState,
    node.key,
    selectedAncestorKeys,
    normalizeFilePath(selectedFilePath ?? ''),
  )
  const visibility = folderVisibility(node.files)

  return (
    <>
      <li
        className={cn(
          TREE_ROW_CLASS,
          'transition-colors hover:bg-accent/50',
          columnsClassName,
        )}
      >
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          <Button
            aria-expanded={!isCollapsed}
            aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${node.name}`}
            onClick={() => onToggleFolder(node.key)}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            {isCollapsed ? (
              <ChevronRight className="size-3" />
            ) : (
              <ChevronDown className="size-3" />
            )}
          </Button>
          {isCollapsed ? (
            <Folder className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.7} />
          ) : (
            <FolderOpen className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.7} />
          )}
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {node.name}
          </span>
        </div>
        <div className="flex items-center gap-1.5 text-xs leading-4 text-muted-foreground">
          {node.files.length} {node.files.length === 1 ? 'file' : 'files'}
        </div>
        {showVisibility && (
          <div className="flex items-center justify-end gap-1.5 sm:justify-center">
            <VisibilityBadge compact visibility={visibility} />
          </div>
        )}
      </li>
      {!isCollapsed &&
        node.children.map((child) => (
          <FileSystemTreeNodeRow
            folderState={folderState}
            columnsClassName={columnsClassName}
            depth={depth + 1}
            getFileMeta={getFileMeta}
            key={child.key}
            node={child}
            onActivateFile={onActivateFile}
            onSelectFile={onSelectFile}
            onToggleFolder={onToggleFolder}
            selectedAncestorKeys={selectedAncestorKeys}
            selectedFilePath={selectedFilePath}
            showVisibility={showVisibility}
          />
        ))}
    </>
  )
}

function folderKeys<TFile extends FileSystemTreeFileBase>(node: FileSystemTreeNode<TFile>): string[] {
  return node.type === 'folder'
    ? [node.key, ...node.children.flatMap((child) => folderKeys(child))]
    : []
}

type FolderState = {
  collapsedForSelection: Map<string, string>
  expanded: Set<string>
}

function folderIsCollapsed(
  state: FolderState,
  key: string,
  selectedAncestorKeys: ReadonlySet<string>,
  selectedPath: string,
) {
  if (state.expanded.has(key)) return false
  return !(
    selectedAncestorKeys.has(key) &&
    state.collapsedForSelection.get(key) !== selectedPath
  )
}

function FilePathLabel({ name, path }: { name: string; path: string }) {
  return (
    <>
      <File className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.7} />
      <span className="min-w-0 truncate font-mono text-xs" title={normalizeFilePath(path)}>
        {name}
      </span>
    </>
  )
}
