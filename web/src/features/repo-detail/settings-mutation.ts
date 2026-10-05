export async function mutateSettings<T>(
  mutation: Promise<T>,
  refresh: () => Promise<unknown>,
  onRefreshError: () => void,
): Promise<T> {
  const result = await mutation
  void Promise.resolve().then(refresh).catch(onRefreshError)
  return result
}
