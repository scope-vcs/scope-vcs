import { displayRouteFilePath } from '../../lib/route-file'

type RepositoryPath = { path: string }

export function matchingRepositoryFiles<T extends RepositoryPath>(files: readonly T[], query: string) {
  const needle = query.trim().toLowerCase()
  return files.filter((file) => displayRouteFilePath(file.path).toLowerCase().includes(needle))
}

export function repositoryResources(files: readonly RepositoryPath[]) {
  const paths = new Map(files.map((file) => [displayRouteFilePath(file.path).toLowerCase(), file.path]))
  return [
    { label: 'License', names: ['license', 'licence'], directories: [''] },
    { label: 'Contributing', names: ['contributing'], directories: ['', '.github/', 'docs/'] },
    { label: 'Security policy', names: ['security'], directories: ['', '.github/', 'docs/'] },
  ].flatMap(({ label, names, directories }) => {
    for (const directory of directories) {
      for (const name of names) {
        for (const extension of ['', '.md', '.html', '.txt']) {
          const path = paths.get(`${directory}${name}${extension}`)
          if (path) return [{ label, path }]
        }
      }
    }
    return []
  })
}
