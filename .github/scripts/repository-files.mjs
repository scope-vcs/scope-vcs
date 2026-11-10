import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

export async function repositoryFiles(root) {
  const { stdout } = await execFileAsync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
    { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
  )
  return stdout.split('\0').filter(Boolean)
}

export async function* repositorySources(root, wanted = () => true) {
  for (const file of await repositoryFiles(root)) {
    if (!wanted(file)) continue
    let contents
    try {
      contents = await readFile(path.join(root, file), 'utf8')
    } catch (error) {
      if (error.code === 'ENOENT' || error.code === 'EISDIR') continue
      throw error
    }
    yield { file, contents }
  }
}
