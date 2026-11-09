import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const sourceRoot = fileURLToPath(new URL('../../', import.meta.url));
const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));

export const RAILWAY_COMPONENTS = Object.keys(manifest.services);
export const BACKEND_COMPONENTS = RAILWAY_COMPONENTS.filter(component => deploymentComponent(component).backend);
export const APPLICATION_COMPONENTS = RAILWAY_COMPONENTS.filter(component => component !== 'cli-downloads');
export const RAILWAY_CONFIG_PATHS = Object.fromEntries(RAILWAY_COMPONENTS
  .filter(component => deploymentComponent(component).verifyTransitionConfig)
  .map(component => [component, deploymentComponent(component).runtimeConfig]));

export function deploymentComponent(component) {
  const definition = manifest.services[component]?.deployment;
  if (!definition) throw new Error(`Unknown release component ${component}.`);
  return definition;
}

export function loadComponentConfig(component, root = sourceRoot) {
  return JSON.parse(readFileSync(resolve(root, deploymentComponent(component).runtimeConfig), 'utf8'));
}

export function backendBinaryCaps(source = manifest) {
  return Object.entries(source.services)
    .filter(([, { deployment }]) => deployment.backend && deployment.artifact.kind === 'binary')
    .flatMap(([component, { deployment: { artifact } }]) => [artifact, artifact.maintenance].filter(Boolean)
      .map(({ binary, maxBytes }) => {
        if (!Number.isSafeInteger(maxBytes) || maxBytes <= 0) throw new Error(`Component ${component} has no positive integer maxBytes for ${binary}.`);
        return { component, binary, maxBytes };
      }));
}

export function backendPrebuiltComponents() {
  return BACKEND_COMPONENTS.filter(name => deploymentComponent(name).artifact.kind === 'binary');
}

export function cargoReleaseBuild(component) {
  const { artifact, runtimeConfig } = deploymentComponent(component);
  if (artifact.kind !== 'binary') throw new Error(`Component ${component} has no prebuilt binary.`);
  const binaries = [
    { source: artifact.sourceBinary, release: artifact.binary },
    ...(artifact.maintenance ? [{ source: artifact.maintenance.binary, release: artifact.maintenance.binary }] : []),
  ];
  return {
    args: ['build', '--release', '--locked', '--manifest-path', `${dirname(runtimeConfig)}/Cargo.toml`,
      ...binaries.flatMap(({ source }) => ['--bin', source])],
    binaries,
  };
}

export function buildBackendComponent(component, outputDirectory, { run = execFileSync, root = sourceRoot } = {}) {
  const { args, binaries } = cargoReleaseBuild(component);
  run('cargo', args, { cwd: root, stdio: 'inherit' });
  const metadata = run('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1'], { cwd: root, encoding: 'utf8' });
  const release = resolve(JSON.parse(metadata).target_directory, 'release');
  mkdirSync(outputDirectory, { recursive: true });
  for (const binary of binaries) {
    const destination = resolve(outputDirectory, binary.release);
    copyFileSync(resolve(release, binary.source), destination);
    chmodSync(destination, 0o755);
  }
}

export function backendSelected(selection) {
  return BACKEND_COMPONENTS.some(component => selection[component] === true);
}

export function serviceIds(manifest) {
  const ids = Object.fromEntries(Object.entries(manifest.services).map(([component, service]) => [component, service.id]));
  ids.postgres = manifest.railway.databaseServiceId;
  ids.maintenance = manifest.railway.maintenanceServiceId;
  for (const [component, id] of Object.entries(ids)) {
    if (!/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(id ?? '')) throw new Error(`Railway deployment requires the ${component} service ID.`);
  }
  return ids;
}

function main() {
  const [command, component, field] = process.argv.slice(2);
  if (command === 'backend-prebuilt') {
    console.log(backendPrebuiltComponents().join('\n'));
    return;
  }
  if (command === 'build') {
    if (!field) throw new Error('usage: deployment-components.mjs build <component> <output-directory>');
    buildBackendComponent(component, resolve(field));
    return;
  }
  const definition = deploymentComponent(component);
  if (command === 'describe') {
    console.log(JSON.stringify(definition));
  } else if (command === 'field') {
    const value = field === 'root' ? dirname(definition.runtimeConfig)
      : field === 'binary' ? definition.artifact.binary
        : definition[field];
    if (typeof value !== 'string' || !value) throw new Error(`Component ${component} has no ${field}.`);
    console.log(value);
  } else {
    throw new Error('usage: deployment-components.mjs describe <component> | field <component> <field> | build <component> <output-directory> | backend-prebuilt');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
