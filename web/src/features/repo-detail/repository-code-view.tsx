import type { RepoContent, RepoParams } from '@/api/types'
import type { RepoFileContentResponse } from '@/api/types.generated'
import { PanelState } from '@/components/empty-state'
import { FileWorkbench } from '@/components/file-workbench'
import { PendingSurface } from '@/components/pending-surface'
import { isRepositoryHtmlPath } from '@/components/repository-html'
import { RepositoryHtmlRenderer, type RepositoryHtmlMode } from '@/components/repository-html-renderer'
import { isRepositoryMarkdownPath } from '@/components/repository-markdown'
import { RepositoryHtmlModeToggle } from '@/components/repository-html-mode-toggle'
import { RepositoryMarkdownRenderer } from '@/components/repository-markdown-renderer'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import { VisibilityBadge } from '@/components/visibility-badge'
import { WorkspaceTabStrip } from '@/components/workspace-tab-strip'
import {
  workspaceTabDomIds,
  workspaceTabPanelId,
  pruneWorkspaceTabs,
  type WorkspaceTabItem,
} from '@/components/workspace-tab-model'
import { formatBytes } from '@/lib/format-bytes'
import {
  displayRouteFilePath,
  routeFileName,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { FileQuestion, Info, TriangleAlert } from 'lucide-react'
import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import {
  readRepositorySourceScroll,
  writeRepositorySourceScroll,
} from './repository-source-scroll-cache'
import {
  FileNavigatorSkeleton,
  SourceCodeSkeleton,
} from './repository-code-skeletons'
import { RepositoryFileNavigator } from './repository-file-navigator'

const CODE_TAB_SET_ID = 'repository-code-files'

export function RepositoryCodeView({
  content,
  contentError,
  contentRetry,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
  workspaceTabs,
}: {
  content: RepoContent | null
  contentError: string | null
  contentRetry: () => void
  onSelectFilePath: (path: string) => void
  params: RepoParams
  selectedFile: RepoFileContentResponse | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
  selectedPath: string | null
  workspaceTabs: ReturnType<typeof useWorkspaceTabs>
}) {
  const [navigationOpen, setNavigationOpen] = useState(false)
  const fileNavigatorRef = useRef<HTMLDivElement>(null)
  const openPath = workspaceTabs.state.openIds.includes(selectedPath ?? '')
    ? selectedPath
    : null
  const visiblePaths = content
    ? content.files.map((file) => displayRouteFilePath(file.path))
    : workspaceTabs.state.openIds
  // An explicit URL keeps its tab and file error even when the tree omits it.
  const availablePaths = selectedPath && !visiblePaths.includes(selectedPath)
    ? [...visiblePaths, selectedPath]
    : visiblePaths

  // Closing the last tab keeps the route pointing at the file it was showing:
  // an empty workspace is this session's state, not something worth sharing.
  function selectFile(path: string, pinned: boolean) {
    workspaceTabs.open(displayRouteFilePath(path), pinned)
    onSelectFilePath(path)
    setNavigationOpen(false)
  }

  return (
    <section>
      <FileWorkbench
        className="lg:min-h-[calc(100dvh-var(--app-chrome))]"
        navigationOpen={navigationOpen}
        onNavigationOpenChange={setNavigationOpen}
        selectedPath={openPath}
      >
        <div
          aria-label="Repository file navigator"
          className="min-w-0 px-2 py-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          ref={fileNavigatorRef}
          tabIndex={-1}
        >
          {content ? (
            <div className="scope-content-enter">
              <RepositoryFileNavigator
                files={content.files}
                onOpenNavigation={() => setNavigationOpen(true)}
                onSelectFile={selectFile}
                selectedPath={selectedRouteFilePath(
                  content.files,
                  openPath ?? undefined,
                )}
              />
            </div>
          ) : contentError ? (
            <FileNavigatorError error={contentError} retry={contentRetry} />
          ) : (
            <PendingSurface
              className="min-h-[220px]"
              delay
              label="Loading repository files"
              onRetry={contentRetry}
              retryLabel="retry files"
            >
              <FileNavigatorSkeleton />
            </PendingSurface>
          )}
        </div>
        <SourcePane
          availablePaths={availablePaths}
          emptyMessage={content && !selectedPath
            ? content.files.length
              ? 'No README in this view. Browse the files or use Find file to get started.'
              : 'Run scope push --main from the CLI to add files to this repository.'
            : 'Select a file to inspect its contents.'}
          error={selectedFileError}
          file={selectedFile}
          loading={selectedFileLoading || (!content && !contentError && !selectedPath)}
          onActivateTab={onSelectFilePath}
          onEmptyTabFocus={() => {
            setNavigationOpen(true)
            requestAnimationFrame(() => fileNavigatorRef.current?.focus())
          }}
          onPinTab={(path) => workspaceTabs.open(path, true)}
          params={params}
          retry={selectedPath ? selectedFileRetry : contentRetry}
          scrollKey={selectedFileIdentity}
          selectedPath={openPath}
          workspaceTabs={workspaceTabs}
        />
      </FileWorkbench>
    </section>
  )
}

function FileNavigatorError({
  error,
  retry,
}: {
  error: string
  retry: () => void
}) {
  return (
    <PanelState role="alert" tone="error">
      <TriangleAlert className="size-5" />
      <span>{error}</span>
      <Button onClick={retry} size="sm" type="button" variant="secondary">
        Retry
      </Button>
    </PanelState>
  )
}

function SourcePane({
  availablePaths,
  emptyMessage,
  error,
  file,
  loading,
  onActivateTab,
  onEmptyTabFocus,
  onPinTab,
  params,
  retry,
  scrollKey,
  selectedPath,
  workspaceTabs,
}: {
  availablePaths: string[]
  emptyMessage: string
  error: string | null
  file: RepoFileContentResponse | null
  loading: boolean
  onActivateTab: (path: string) => void
  onEmptyTabFocus: () => void
  onPinTab: (path: string) => void
  params: RepoParams
  retry: () => void
  scrollKey: string | null
  selectedPath: string | null
  workspaceTabs: ReturnType<typeof useWorkspaceTabs>
}) {
  const activeTabDomIds = selectedPath
    ? workspaceTabDomIds(CODE_TAB_SET_ID, selectedPath)
    : null
  const contentRef = useRef<HTMLDivElement>(null)
  const fileIdentity = selectedPath && file ? `${file.path}:${file.oid}` : selectedPath
  const [display, setDisplay] = useState<{ identity: string | null; mode: RepositoryHtmlMode }>({
    identity: fileIdentity,
    mode: 'preview',
  })
  if (display.identity !== fileIdentity) {
    setDisplay({ identity: fileIdentity, mode: 'preview' })
  }
  const htmlMode = display.identity === fileIdentity ? display.mode : 'preview'
  const meta = file && selectedPath && !loading && !error ? (
    <>
      {file.content.kind === 'text' && isRepositoryHtmlPath(file.path) && (
        <RepositoryHtmlModeToggle
          mode={htmlMode}
          onSelect={(mode) => setDisplay({ identity: fileIdentity, mode })}
          path={file.path}
        />
      )}
      <FileMeta file={file} />
    </>
  ) : undefined

  useLayoutEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTop = readRepositorySourceScroll(scrollKey)
    }
  }, [scrollKey])

  return (
    <div className="min-w-0">
      <RepositoryTabStrip
        availablePaths={availablePaths}
        meta={meta}
        onActivateTab={onActivateTab}
        onEmptyTabFocus={onEmptyTabFocus}
        onPinTab={onPinTab}
        selectedPath={selectedPath}
        workspaceTabs={workspaceTabs}
      />
      <div
        aria-label={activeTabDomIds ? undefined : 'Repository file viewer'}
        aria-labelledby={activeTabDomIds?.tabId}
        className="max-h-[calc(100dvh-var(--app-chrome)-84px)] overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        id={workspaceTabPanelId(CODE_TAB_SET_ID)}
        onScroll={(event) =>
          writeRepositorySourceScroll(scrollKey, event.currentTarget.scrollTop)
        }
        ref={contentRef}
        role={selectedPath ? 'tabpanel' : undefined}
        tabIndex={selectedPath ? 0 : undefined}
      >
        <SourceContent
          htmlMode={htmlMode}
          emptyMessage={emptyMessage}
          error={error}
          file={file}
          loading={loading}
          params={params}
          retry={retry}
          selectedPath={selectedPath}
        />
      </div>
    </div>
  )
}

function RepositoryTabStrip({
  availablePaths,
  meta,
  onActivateTab,
  onEmptyTabFocus,
  onPinTab,
  selectedPath,
  workspaceTabs,
}: {
  availablePaths: string[]
  meta: ReactNode
  onActivateTab: (path: string) => void
  onEmptyTabFocus: () => void
  onPinTab: (path: string) => void
  selectedPath: string | null
  workspaceTabs: ReturnType<typeof useWorkspaceTabs>
}) {
  const items = availablePaths.map(workspaceTabItem)
  const itemById = new Map(items.map((item) => [item.id, item]))
  const availableIds = new Set(itemById.keys())
  const openState = pruneWorkspaceTabs(workspaceTabs.state, availableIds)
  const tabs = openState.openIds.flatMap((id) => {
    const item = itemById.get(id)
    return item ? [item] : []
  })

  function closeTab(id: string) {
    const result = workspaceTabs.close(id, availableIds)
    if (id === selectedPath && result.activeId) onActivateTab(result.activeId)
    return result.focusId
  }

  return (
    <WorkspaceTabStrip
      activeId={selectedPath}
      ariaLabel="Open repository files"
      meta={meta}
      onActivate={onActivateTab}
      onClose={closeTab}
      onEmptyFocus={onEmptyTabFocus}
      onPin={onPinTab}
      previewId={openState.previewId}
      tabSetId={CODE_TAB_SET_ID}
      tabs={tabs}
    />
  )
}

function SourceContent({
  htmlMode,
  emptyMessage,
  error,
  file,
  loading,
  params,
  retry,
  selectedPath,
}: {
  emptyMessage: string
  error: string | null
  file: RepoFileContentResponse | null
  loading: boolean
  params: RepoParams
  retry: () => void
  selectedPath: string | null
  htmlMode: RepositoryHtmlMode
}) {
  if (loading) {
    return (
      <PendingSurface
        className="min-h-[220px]"
        delay
        label={selectedPath ? `Loading ${displayRouteFilePath(selectedPath)}` : 'Loading repository introduction'}
        delayedLabel="this file is taking longer than usual"
        key={selectedPath ?? 'introduction'}
        onRetry={retry}
        retryLabel="retry file"
      >
        <SourceCodeSkeleton />
      </PendingSurface>
    )
  }

  if (!selectedPath) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>{emptyMessage}</span>
      </PanelState>
    )
  }

  if (error) {
    return (
      <PanelState role="alert" tone="error">
        <TriangleAlert className="size-5" />
        <span>{error}</span>
        <Button onClick={retry} size="sm" type="button" variant="secondary">
          Retry
        </Button>
      </PanelState>
    )
  }

  if (!file) {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>This file is no longer available in the current scoped view.</span>
      </PanelState>
    )
  }

  return (
    <div className="scope-content-enter min-h-full" key={file.oid}>
      <SourceFileContent file={file} htmlMode={htmlMode} params={params} />
    </div>
  )
}

function FileMeta({ file }: { file: RepoFileContentResponse }) {
  const [open, setOpen] = useState(false)

  return (
    <>
      <TooltipProvider>
        <Tooltip onOpenChange={setOpen} open={open}>
          <TooltipTrigger
            aria-label="File details"
            className="flex cursor-pointer items-center rounded p-1 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
            onClick={(event) => {
              event.preventDefault()
              setOpen(!open)
            }}
          >
            <Info aria-hidden="true" className="size-3.5" />
          </TooltipTrigger>
          <TooltipContent
            align="end"
            className="w-[min(280px,calc(100vw-3rem))] border border-border bg-popover p-3 text-popover-foreground shadow-[var(--shadow-pop)]"
            side="bottom"
          >
            <p>{formatBytes(file.size_bytes)}</p>
            <p className="mt-1 break-all font-mono">Blob: {file.oid}</p>
            {isRepositoryHtmlPath(file.path) && (
              <p className="mt-2 text-muted-foreground">
                Sandboxed document. Repository HTML runs in an isolated preview.
              </p>
            )}
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
      <VisibilityBadge compact visibility={file.visibility} />
    </>
  )
}

function SourceFileContent({
  file,
  htmlMode,
  params,
}: {
  file: RepoFileContentResponse
  htmlMode: RepositoryHtmlMode
  params: RepoParams
}) {
  if (file.content.kind !== 'text') {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>
          Binary file not rendered ·{' '}
          {formatBytes(file.content.size_bytes)} ·{' '}
          {file.content.oid.slice(0, 12)}
        </span>
      </PanelState>
    )
  }

  if (isRepositoryMarkdownPath(file.path)) {
    return (
      <RepositoryMarkdownRenderer
        repository={{ ...params, markdownPath: file.path }}
        source={file.content.text}
      />
    )
  }

  if (isRepositoryHtmlPath(file.path)) {
    return (
      <RepositoryHtmlRenderer
        identity={`${file.path}\0${file.oid}`}
        key={`${file.path}:${file.oid}`}
        path={file.path}
        mode={htmlMode}
        source={file.content.text}
      />
    )
  }

  return (
    <pre className="min-h-full bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
      <code>{file.content.text}</code>
    </pre>
  )
}

function workspaceTabItem(path: string): WorkspaceTabItem {
  return {
    id: path,
    label: routeFileName(path),
    title: displayRouteFilePath(path),
  }
}
