import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const toolchain = (path) => readFileSync(path, 'utf8').match(/^channel\s*=\s*"([^"]+)"/m)[1];

export function smokeToolsEvidence(directory, sourceSha, toolchainPath) {
  if (!/^[a-f0-9]{40}$/.test(sourceSha ?? '')) throw new Error('Smoke tools require an exact source SHA');
  return { sourceSha, rustToolchain: toolchain(toolchainPath), archiveSha256: digest(readFileSync(join(directory, 'staging-commands.tar.gz'))) };
}

export function verifySmokeTools(directory, sourceSha, toolchainPath) {
  const expected = smokeToolsEvidence(directory, sourceSha, toolchainPath);
  const actual = JSON.parse(readFileSync(join(directory, 'staging-commands.json'), 'utf8'));
  for (const key of Object.keys(expected)) {
    if (actual[key] !== expected[key]) throw new Error(`Smoke tools ${key} mismatch: expected ${expected[key]}, found ${actual[key]}`);
  }
  return expected;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [command, directory, sourceSha, toolchainPath] = process.argv.slice(2);
  if (command === 'create') writeFileSync(join(directory, 'staging-commands.json'), `${JSON.stringify(smokeToolsEvidence(directory, sourceSha, toolchainPath))}\n`);
  else if (command === 'verify') console.log(`Verified smoke tools: ${JSON.stringify(verifySmokeTools(directory, sourceSha, toolchainPath))}`);
  else throw new Error('Usage: smoke-tools-evidence.mjs create|verify DIRECTORY SOURCE_SHA SOURCE_TOOLCHAIN_FILE');
}
