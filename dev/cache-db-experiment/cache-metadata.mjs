// Experiment only: recover cache metadata without changing the deployed runner.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { lstatSync, readFileSync, readdirSync, readlinkSync, existsSync, utimesSync, lutimesSync, writeFileSync } from 'node:fs';
import { resolve, relative, join } from 'node:path';

const metadataName = '.scope-experiment-metadata.json';
const fingerprint = (path, stat) => stat.isSymbolicLink()
  ? `link:${readlinkSync(path)}`
  : createHash('sha256').update(readFileSync(path)).digest('hex');

export function sourceSnapshot(root) {
  const paths = execFileSync('git', ['ls-files', '-z'], { cwd: root }).toString().split('\0').filter(Boolean);
  return paths.map((path) => {
    const full = join(root, path);
    const stat = lstatSync(full);
    return { path, hash: fingerprint(full, stat), mtime: stat.mtimeMs / 1000, link: stat.isSymbolicLink() };
  });
}

export function saveMetadata(root, target, sources) {
  const outputs = [];
  function visit(directory) {
    for (const name of readdirSync(directory)) {
      if (directory === target && name === metadataName) continue;
      const path = join(directory, name);
      const stat = lstatSync(path);
      outputs.push({ path: relative(target, path), mtime: stat.mtimeMs / 1000, link: stat.isSymbolicLink() });
      if (stat.isDirectory()) visit(path);
    }
  }
  visit(target);
  writeFileSync(join(target, metadataName), JSON.stringify({ root, sources, outputs }));
  return { sources: sources.length, outputs: outputs.length };
}

export function restoreMetadata(root, target, mode) {
  if (!['baseline', 'timestamps-only', 'content'].includes(mode)) throw new Error(`Unknown cache experiment mode: ${mode}`);
  const file = join(target, metadataName);
  if (!existsSync(file) || mode === 'baseline') return { restored: false };
  const metadata = JSON.parse(readFileSync(file));
  if (metadata.root !== root) throw new Error('Experiment requires the same absolute workspace path');
  let sources = 0;
  let changed = 0;
  const restore = (base, entry) => {
    const path = resolve(base, entry.path);
    if (!path.startsWith(`${resolve(base)}/`)) throw new Error('Metadata path escapes its root');
    if (!existsSync(path)) return;
    (entry.link ? lutimesSync : utimesSync)(path, entry.mtime, entry.mtime);
  };
  for (const entry of metadata.outputs) restore(target, entry);
  if (mode === 'content') {
    const newestOutput = Math.max(...metadata.outputs.map((entry) => entry.mtime));
    for (const entry of metadata.sources) {
      const path = resolve(root, entry.path);
      if (!path.startsWith(`${root}/`)) throw new Error('Source metadata path escapes workspace');
      if (!existsSync(path)) { changed++; continue; }
      if (fingerprint(path, lstatSync(path)) !== entry.hash) {
        // Content changed: keep it newer even if the saved cache came from a faster clock.
        const stat = lstatSync(path);
        if (stat.mtimeMs / 1000 <= newestOutput) {
          const changedMtime = Math.max(Date.now() / 1000, newestOutput + 0.001);
          (stat.isSymbolicLink() ? lutimesSync : utimesSync)(path, changedMtime, changedMtime);
        }
        changed++;
        continue;
      }
      restore(root, entry);
      sources++;
    }
  }
  return { restored: true, outputs: metadata.outputs.length, unchangedSources: sources, changedSources: changed };
}
