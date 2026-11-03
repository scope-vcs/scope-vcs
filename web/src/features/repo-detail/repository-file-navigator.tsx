import { FileSystemTree } from '@/components/file-system-tree'
import type { RepoFileResponse } from '@/api/types.generated'

export function RepositoryFileNavigator({
  files,
  onSelectFile,
  selectedPath,
  showVisibility,
}: {
  files: RepoFileResponse[]
  onSelectFile: (path: string, pinned: boolean) => void
  selectedPath: string | null
  showVisibility: boolean
}) {
  if (files.length === 0) {
    return <p className="px-2 py-4 text-xs text-muted-foreground">No files yet.</p>
  }

  return (
    <>
      <div className="mb-2 flex items-baseline justify-between gap-2 px-2 text-xs">
        <h2 className="font-medium">Files</h2>
        <span className="text-muted-foreground">{files.length}</span>
      </div>
      <FileSystemTree
        files={files}
        onActivateFile={(file) => onSelectFile(file.path, true)}
        onSelectFile={(file) => onSelectFile(file.path, false)}
        selectedFilePath={selectedPath}
        showVisibility={showVisibility}
      />
    </>
  )
}
