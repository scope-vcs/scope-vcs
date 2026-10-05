const KILOBYTE = 1024

export function formatBytes(bytes: number) {
  if (bytes < KILOBYTE) return `${bytes} B`
  if (bytes < KILOBYTE ** 2) return `${(bytes / KILOBYTE).toFixed(1)} KB`
  if (bytes < KILOBYTE ** 3) return `${(bytes / KILOBYTE ** 2).toFixed(1)} MB`
  return `${(bytes / KILOBYTE ** 3).toFixed(1)} GB`
}
