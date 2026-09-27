import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { loadDeploymentManifest, validatePreparedRelease } from './railway-artifact.mjs';

export function mergePreparedReleaseFragments(fragments, { sourceSha, components, services }) {
  if (!Array.isArray(components) || components.length === 0 || new Set(components).size !== components.length) {
    throw new Error('Expected release components must be distinct and nonempty.');
  }
  if (fragments.length !== components.length) throw new Error('Every selected component requires one prepared fragment.');
  const expected = new Set(components);
  const merged = { schemaVersion: 1, sourceSha, components: {} };
  let metadata;
  for (const fragment of fragments) {
    validatePreparedRelease(fragment, { sourceSha, services });
    const entries = Object.entries(fragment.components);
    if (entries.length !== 1) throw new Error('Each prepared fragment must contain exactly one component.');
    const [component, artifact] = entries[0];
    if (!expected.has(component)) throw new Error(`Unexpected or duplicate prepared component ${component}.`);
    expected.delete(component);
    const currentMetadata = {
      preparationRunId: fragment.preparationRunId,
      maintenanceSha256: fragment.maintenanceSha256,
    };
    if (metadata && (metadata.preparationRunId !== currentMetadata.preparationRunId ||
        metadata.maintenanceSha256 !== currentMetadata.maintenanceSha256)) {
      throw new Error('Prepared fragments disagree on release metadata.');
    }
    metadata = currentMetadata;
    merged.components[component] = artifact;
  }
  if (expected.size) throw new Error(`Prepared release is missing ${[...expected].join(', ')}.`);
  if (components.includes('api') && metadata.maintenanceSha256 === undefined) {
    throw new Error('Prepared API release is missing its maintenance binary digest.');
  }
  if (metadata.preparationRunId !== undefined) merged.preparationRunId = metadata.preparationRunId;
  if (metadata.maintenanceSha256 !== undefined) merged.maintenanceSha256 = metadata.maintenanceSha256;
  return validatePreparedRelease(merged, { sourceSha, components, services });
}

function main() {
  const [output, sourceSha, componentList, ...paths] = process.argv.slice(2);
  if (!output || !sourceSha || !componentList) {
    throw new Error('usage: merge-prepared-release.mjs <output> <source-sha> <space-separated-components> <fragments...>');
  }
  const { services } = loadDeploymentManifest();
  const release = mergePreparedReleaseFragments(paths.map(path => JSON.parse(readFileSync(path, 'utf8'))), {
    sourceSha, components: componentList.split(' '), services,
  });
  if (process.env.GITHUB_RUN_ID && release.preparationRunId !== process.env.GITHUB_RUN_ID) {
    throw new Error('Prepared fragments do not match this preparation run.');
  }
  const temporary = `${output}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(release, null, 2)}\n`);
  renameSync(temporary, output);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
