import { execFile } from 'node:child_process'
import { access, readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

const execFileAsync = promisify(execFile)
const applicationPackages = new Set(['api', 'scope-cache-service', 'scope-repo-router', 'scope-runner-runtime', 'worker'])
const leafPackages = new Set(['scope-cache-domain', 'scope-domain', 'scope-git-process'])
const contractDependencies = new Map([
  ['scope-api-contract', new Set(['scope-domain'])],
  ['scope-cache-contract', new Set(['scope-cache-domain'])],
])
function workspacePackages(metadata) {
  const members = new Set(metadata.workspace_members)
  return metadata.packages.filter(({ id }) => members.has(id))
}

function normalWorkspaceDependencies(pkg, workspaceNames) {
  return pkg.dependencies
    .filter(({ kind, name, source }) => kind !== 'dev' && source === null && workspaceNames.has(name))
    .map(({ name }) => name)
    .sort()
}

export function validateRustBoundaries(mainMetadata, cliMetadata, expectedCliRoot) {
  const errors = []
  const packages = workspacePackages(mainMetadata)
  const packageByName = new Map(packages.map((pkg) => [pkg.name, pkg]))
  const workspaceNames = new Set(packageByName.keys())
  const dependencies = new Map(packages.map((pkg) => [
    pkg.name,
    normalWorkspaceDependencies(pkg, workspaceNames),
  ]))

  for (const leaf of leafPackages) {
    const actual = dependencies.get(leaf)
    if (!actual) errors.push(`${leaf}: required leaf package is missing from the main workspace`)
    else if (actual.length > 0) errors.push(`${leaf}: domain leaf depends on ${actual.join(', ')}`)
  }

  for (const [contract, allowed] of contractDependencies) {
    const actual = dependencies.get(contract)
    if (!actual) {
      errors.push(`${contract}: required contract package is missing from the main workspace`)
      continue
    }
    const forbidden = actual.filter((dependency) => !allowed.has(dependency))
    if (forbidden.length > 0) errors.push(`${contract}: contract depends on ${forbidden.join(', ')}`)
  }

  for (const [name, actual] of dependencies) {
    if (applicationPackages.has(name)) continue
    const forbidden = actual.filter((dependency) => applicationPackages.has(dependency))
    if (forbidden.length > 0) errors.push(`${name}: reusable package depends on application ${forbidden.join(', ')}`)
  }

  const lifecycleConsumers = [...dependencies]
    .filter(([, actual]) => actual.includes('scope-content-lifecycle'))
    .map(([name]) => name)
    .sort()
  if (lifecycleConsumers.length < 2 || !lifecycleConsumers.includes('api') || !lifecycleConsumers.includes('worker')) {
    errors.push(`scope-content-lifecycle: expected API and worker consumers, found ${lifecycleConsumers.join(', ') || 'none'}`)
  }

  const cliPackages = workspacePackages(cliMetadata)
  if (path.resolve(cliMetadata.workspace_root) !== path.resolve(expectedCliRoot)) {
    errors.push(`scope-cli: workspace root must remain ${path.resolve(expectedCliRoot)}`)
  }
  if (cliPackages.length !== 1 || cliPackages[0]?.name !== 'scope-cli') {
    errors.push('scope-cli: standalone workspace must contain only scope-cli')
  } else {
    const allowed = new Set(['scope-api-contract', 'scope-domain'])
    const actual = cliPackages[0].dependencies
      .filter(({ kind, source }) => kind !== 'dev' && source === null)
      .map(({ name }) => name)
    const forbidden = actual.filter((dependency) => !allowed.has(dependency))
    if (forbidden.length > 0) errors.push(`scope-cli: unexpected local dependency ${forbidden.join(', ')}`)
  }

  return { dependencies, errors, lifecycleConsumers }
}

export function validateStandaloneManifests(manifests) {
  const errors = []
  for (const [name, contents] of Object.entries(manifests)) {
    if (/\bworkspace\s*=\s*true\b|\b(?:version|edition|license|repository)\.workspace\s*=\s*true\b/.test(contents)) {
      errors.push(`${name}: manifest must stay self-contained for standalone CLI staging`)
    }
  }
  return errors
}

async function cargoMetadata(root, manifestPath) {
  const { stdout } = await execFileAsync(
    'cargo',
    ['metadata', '--manifest-path', manifestPath, '--format-version', '1', '--no-deps', '--locked'],
    { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
  )
  return JSON.parse(stdout)
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const cliRoot = path.join(root, 'cli')
  const [mainMetadata, cliMetadata] = await Promise.all([
    cargoMetadata(root, path.join(root, 'Cargo.toml')),
    cargoMetadata(root, path.join(cliRoot, 'Cargo.toml')),
  ])
  const standaloneManifests = Object.fromEntries(await Promise.all([
    ['scope-api-contract', path.join(root, 'crates', 'scope-api-contract', 'Cargo.toml')],
    ['scope-domain', path.join(root, 'crates', 'scope-domain', 'Cargo.toml')],
  ].map(async ([name, manifest]) => [name, await readFile(manifest, 'utf8')])))
  await access(path.join(cliRoot, 'Cargo.lock'))
  const result = validateRustBoundaries(mainMetadata, cliMetadata, cliRoot)
  result.errors.push(...validateStandaloneManifests(standaloneManifests))
  if (result.errors.length > 0) {
    console.error(`Rust dependency boundary check failed:\n- ${result.errors.join('\n- ')}`)
    process.exitCode = 1
    return
  }
  console.log(`Rust dependency boundaries pass; scope-content-lifecycle consumers: ${result.lifecycleConsumers.join(', ')}.`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
