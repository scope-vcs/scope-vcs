const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function railwayEnvironmentInventory(manifest, project) {
  const protectedIds = ['production', 'staging'].map(name => manifest.environments?.[name]?.environmentId);
  if (protectedIds.some(id => !uuid.test(id ?? '')) || new Set(protectedIds).size !== 2) {
    throw new Error('Manifest must identify distinct production and staging environments');
  }
  if (project?.id !== manifest.railway?.projectId || !Array.isArray(project.environments?.edges)) {
    throw new Error('Railway response must identify the manifest project and its environments');
  }
  if (project.environments.pageInfo?.hasNextPage !== false) {
    throw new Error('Railway environment inventory is incomplete; all environments must be audited');
  }
  const environments = project.environments.edges.map(({ node }) => node);
  const liveIds = new Set();
  for (const environment of environments) {
    if (!uuid.test(environment?.id ?? '') || liveIds.has(environment.id)) throw new Error('Railway returned an invalid or duplicate environment ID');
    liveIds.add(environment.id);
  }
  return { environments, protectedIds, liveIds };
}
