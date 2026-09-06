export function historySelectedFilePath(
  path: string | undefined,
  files: readonly { path: string }[] | undefined,
  dismissed: boolean,
): string | null {
  if (dismissed) return null
  return path ?? files?.[0]?.path ?? null
}
