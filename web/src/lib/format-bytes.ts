const KILOBYTE = 1024

/** Binary-scaled bytes with one decimal: 512 B, 1.5 KB, 12.0 MB, 1.0 GB. */
export function formatBytes(bytes: number) {
  if (bytes < KILOBYTE) return `${bytes} B`
  if (bytes < KILOBYTE ** 2) return `${(bytes / KILOBYTE).toFixed(1)} KB`
  if (bytes < KILOBYTE ** 3) return `${(bytes / KILOBYTE ** 2).toFixed(1)} MB`
  return `${(bytes / KILOBYTE ** 3).toFixed(1)} GB`
}
