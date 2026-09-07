import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

// Resolve only repository-reviewed targets before issuing any Railway credentials.
// The deployment helpers consume the selected target through their existing manifest.
export function selectProofEnvironment(manifest, name) {
  assert(name === 'staging' || Object.hasOwn(manifest.railway.proofEnvironments ?? {}, name),
    'Unknown proof environment')
  const selected = name === 'staging'
    ? manifest.railway.staging
    : manifest.railway.proofEnvironments[name]
  assert(selected.environmentName === name, 'Proof environment name does not match')
  assert(selected.environmentId && selected.environmentId !== manifest.railway.environmentId,
    'Proof environment must differ from production')
  if (name === 'staging') return manifest
  assert(selected.environmentId !== manifest.railway.staging.environmentId,
    'Separate proof environment must differ from shared staging')
  const { mediaGatewayDomain, ...staging } = selected
  assert(typeof mediaGatewayDomain === 'string' && mediaGatewayDomain.length > 0,
    'Proof media gateway domain is required')
  return {
    ...manifest,
    railway: { ...manifest.railway, staging },
    mediaResources: {
      ...manifest.mediaResources,
      staging: { ...manifest.mediaResources.staging, gatewayDomain: mediaGatewayDomain },
    },
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const path = '.github/deployment-services.json'
  const manifest = JSON.parse(readFileSync(path, 'utf8'))
  writeFileSync(path, `${JSON.stringify(selectProofEnvironment(manifest, process.argv[2]), null, 2)}\n`)
}
