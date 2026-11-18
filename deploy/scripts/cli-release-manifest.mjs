#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export function cliReleaseTag(sourceSha) {
  if (!/^[0-9a-f]{40}$/.test(sourceSha ?? '')) throw new Error('A full source revision is required.');
  return `cli-${sourceSha.slice(0, 12)}`;
}

export function cliReleaseManifest(sourceSha, targets, read) {
  return {
    tag: cliReleaseTag(sourceSha),
    sourceSha,
    artifacts: Object.fromEntries(targets.map(({ artifact }) => {
      const bytes = read(artifact);
      if (bytes.length === 0) throw new Error(`CLI artifact ${artifact} is empty.`);
      return [artifact, createHash('sha256').update(bytes).digest('hex')];
    })),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [sourceSha, artifactDirectory, output] = process.argv.slice(2);
  if (!artifactDirectory) {
    process.stdout.write(`${cliReleaseTag(sourceSha)}\n`);
    process.exit(0);
  }
  if (!output) throw new Error('usage: cli-release-manifest.mjs <source-sha> [<artifact-directory> <output>]');
  const { targets } = JSON.parse(readFileSync(new URL('../../cli/distribution/targets.json', import.meta.url), 'utf8'));
  const manifest = cliReleaseManifest(sourceSha, targets, (artifact) => readFileSync(join(artifactDirectory, artifact)));
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
}
