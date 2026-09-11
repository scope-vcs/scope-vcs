export type RepositoryMetadata = { description: string; website_url: string }
export type MetadataDraft = {
  source: RepositoryMetadata
  value: RepositoryMetadata
  conflict: boolean
}

export function repositoryMetadata(value: { description: string | null; website_url: string | null }): RepositoryMetadata {
  return { description: value.description ?? '', website_url: value.website_url ?? '' }
}

export function sameMetadata(left: RepositoryMetadata, right: RepositoryMetadata) {
  return left.description === right.description && left.website_url === right.website_url
}

export function reconcileMetadataDraft(current: MetadataDraft, incoming: RepositoryMetadata): MetadataDraft {
  if (sameMetadata(current.source, incoming)) return current
  const dirty = !sameMetadata(current.source, current.value)
  return {
    source: incoming,
    value: dirty ? current.value : incoming,
    conflict: dirty && !sameMetadata(current.value, incoming),
  }
}
