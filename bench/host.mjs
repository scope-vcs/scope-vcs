import { readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';

export function message(error) {
  return error instanceof Error ? error.message : String(error);
}

export async function directoryBytes(path) {
  let total = 0;
  const pending = [path];
  while (pending.length) {
    const current = pending.pop();
    const info = await stat(current);
    if (info.isDirectory()) for (const entry of await readdir(current)) pending.push(join(current, entry));
    else total += info.size;
  }
  return total;
}
