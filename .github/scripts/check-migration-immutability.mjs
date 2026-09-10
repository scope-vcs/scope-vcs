import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const directory = 'crates/scope-postgres/src/migrations';
const lockPath = `${directory}/sources.lock.json`;
export const migrationDigest = (contents) => createHash('sha256').update(contents).digest('hex');
const isDefinition = (name) => /^m\d+_.*\.rs$/.test(name)
  || name === 'current_schema.sql' || name === 'baseline_ledger.txt';

export function checkMigrationSources(sources, locked, previous = {}) {
  const errors = [];
  for (const [name, digest] of Object.entries(previous)) {
    if (locked[name] !== digest) errors.push(`${name}: an existing migration lock cannot change or disappear; add a new migration`);
  }
  for (const [name, digest] of Object.entries(locked)) {
    if (!isDefinition(name) || !/^[a-f0-9]{64}$/.test(digest)) errors.push(`${name}: invalid migration lock entry`);
    else if (sources[name] === undefined) errors.push(`${name}: locked migration source is missing`);
    else if (migrationDigest(sources[name]) !== digest) errors.push(`${name}: migration definition changed; add a new migration`);
  }
  for (const name of Object.keys(sources)) {
    if (!(name in locked)) errors.push(`${name}: new migration needs a SHA-256 entry in ${lockPath}`);
  }
  return errors;
}

function previousLock() {
  // PR checks compare the lock itself against the target branch, so changing
  // both a historical migration and its checksum cannot conceal the edit.
  const event = process.env.GITHUB_EVENT_PATH
    ? JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8')) : {};
  const base = event.pull_request?.base?.sha;
  if (!base) return {};
  if (!/^[a-f0-9]{40}$/.test(base)) throw new Error('Invalid pull request base SHA');
  const entry = execFileSync('git', ['ls-tree', base, '--', lockPath], { encoding: 'utf8' });
  if (!entry.trim()) return {}; // Initial adoption of the migration lock.
  return JSON.parse(execFileSync('git', ['show', `${base}:${lockPath}`], { encoding: 'utf8' }));
}

function main() {
  const sources = Object.fromEntries(readdirSync(directory).filter(isDefinition)
    .map((name) => [name, readFileSync(`${directory}/${name}`)]));
  const errors = checkMigrationSources(sources, JSON.parse(readFileSync(lockPath, 'utf8')), previousLock());
  if (errors.length) throw new Error(`Migration sources are immutable:\n${errors.join('\n')}`);
  process.stdout.write(`Verified ${Object.keys(sources).length} immutable migration definitions.\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
