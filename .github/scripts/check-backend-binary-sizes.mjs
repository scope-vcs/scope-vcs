#!/usr/bin/env node
import { appendFileSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { backendBinaryCaps } from './deployment-components.mjs';

const [directory] = process.argv.slice(2);
if (!directory) throw new Error('usage: check-backend-binary-sizes.mjs <binary-directory>');
const manifestPath = process.env.SCOPE_DEPLOYMENT_MANIFEST;
const caps = new Map(backendBinaryCaps(manifestPath ? JSON.parse(readFileSync(manifestPath, 'utf8')) : undefined)
  .map(({ binary, maxBytes }) => [binary, maxBytes]));
const built = readdirSync(directory).sort();
const failures = [
  ...built.filter(binary => !caps.has(binary)).map(binary => `${binary} has no size cap in the deployment manifest`),
  ...[...caps.keys()].filter(binary => !built.includes(binary)).map(binary => `${binary} was not built`),
];
const table = ['| Binary | Bytes | MiB | Cap | Over |', '| --- | ---: | ---: | ---: | ---: |'];
for (const binary of built.filter(name => caps.has(name))) {
  const bytes = statSync(join(directory, binary)).size;
  const cap = caps.get(binary);
  const over = Math.max(0, bytes - cap);
  table.push(`| \`${binary}\` | ${bytes} | ${(bytes / 1048576).toFixed(1)} | ${cap} | ${over} |`);
  if (over) failures.push(`${binary} is ${bytes} bytes, over the ${cap} byte cap by ${over} bytes`);
}

console.log(table.join('\n'));
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### Backend release binary sizes\n\n${table.join('\n')}\n`);
}
for (const failure of failures) console.error(failure);
if (failures.length) process.exitCode = 1;
