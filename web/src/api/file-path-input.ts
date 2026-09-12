export function parseFilePath(value: unknown): string {
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('path must be a string that is not empty.')
  }
  if (new TextEncoder().encode(value).length > 4096) {
    throw new Error('path exceeds 4096 bytes.')
  }
  if (value.includes('\0')) throw new Error('path contains a NUL byte.')
  return value
}
