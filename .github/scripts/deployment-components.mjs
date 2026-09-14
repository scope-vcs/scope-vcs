import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const sourceRoot = fileURLToPath(new URL('../../', import.meta.url));
const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));

// These are source-owned deployment facts. Target manifests may override service
// IDs and environment settings, but preparation and activation use this contract.
export const RAILWAY_COMPONENTS = Object.keys(manifest.services);
export const BACKEND_COMPONENTS = RAILWAY_COMPONENTS.filter(component => deploymentComponent(component).backend);
export const APPLICATION_COMPONENTS = RAILWAY_COMPONENTS.filter(component => deploymentComponent(component).artifact.kind !== 'upload');
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

export function backendSelected(selection) {
  return BACKEND_COMPONENTS.some(component => selection[component] === true);
}

function main() {
  const [command, component, field] = process.argv.slice(2);
  if (command === 'backend-prebuilt') {
    console.log(BACKEND_COMPONENTS.filter(name => deploymentComponent(name).artifact.kind === 'binary').join('\n'));
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
    throw new Error('usage: deployment-components.mjs describe <component> | field <component> <field> | backend-prebuilt');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
