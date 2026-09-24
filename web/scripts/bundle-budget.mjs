import { readdir } from 'node:fs/promises'
import { join } from 'node:path'

export async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  return (await Promise.all(entries.map(async (entry) => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? filesBelow(path) : [path]
  }))).flat()
}

export function sumBytes(sizes) {
  return sizes.reduce((total, size) => total + size, 0)
}
