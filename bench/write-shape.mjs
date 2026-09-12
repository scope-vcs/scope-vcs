import { randomBytes } from 'node:crypto';
import { mkdir, open, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export function parseChangedFileCounts(value) {
  const entries = value.split(',').map((entry) => entry.trim());
  const counts = [...new Set(entries.map((entry) => /^\d+$/.test(entry) ? Number(entry) : Number.NaN))];
  if (!counts.length || counts.some((count) => !Number.isSafeInteger(count) || count < 0)) {
    throw new Error('SCOPE_LOAD_CHANGED_FILE_COUNTS must be a comma-separated list of non-negative integers');
  }
  return counts.sort((left, right) => left - right);
}

export async function writeChangedFiles(directory, count, bytesPerFile, update) {
  if (![count, bytesPerFile, update].every(Number.isSafeInteger)
    || count < 0 || bytesPerFile < 1 || update < 0) {
    throw new Error('changed-file fixture values must be safe integers with positive file size');
  }
  await mkdir(directory, { recursive: true });
  const content = Buffer.alloc(bytesPerFile, 'x');
  const marker = Buffer.from(`scope-load-update:${update}\n`);
  marker.copy(content, 0, 0, Math.min(marker.length, content.length));
  for (let offset = 0; offset < count; offset += 64) {
    await Promise.all(Array.from({ length: Math.min(64, count - offset) }, (_, index) => writeFile(
      join(directory, `${String(offset + index).padStart(6, '0')}.txt`),
      content,
    )));
  }
}

export const WRITE_DELTA_FILE_BYTES = 16 * 1024 * 1024;
const RANDOM_WRITE_BUFFER_BYTES = 256 * 1024;

export async function writeSeedPayload(directory, bytes) {
  const payloadDir = join(directory, 'fixture');
  await mkdir(payloadDir, { recursive: true });
  let remaining = bytes;
  let index = 0;
  while (remaining > 0) {
    const size = Math.min(remaining, 256 * 1024);
    await writeFile(join(payloadDir, `${String(index++).padStart(4, '0')}.bin`), randomBytes(size));
    remaining -= size;
  }
}

export async function writeChunkedRandomPayload(
  directory,
  bytes,
  maxFileBytes = WRITE_DELTA_FILE_BYTES,
  bufferBytes = RANDOM_WRITE_BUFFER_BYTES,
) {
  if (![bytes, maxFileBytes, bufferBytes].every(Number.isSafeInteger)
    || bytes < 0 || maxFileBytes < 1 || bufferBytes < 1) {
    throw new Error('chunked payload sizes must be non-negative safe integers with positive chunk limits');
  }
  await mkdir(directory, { recursive: true });
  const paths = [];
  let remaining = bytes;
  let fileIndex = 0;
  while (remaining > 0) {
    const path = join(directory, `${String(fileIndex++).padStart(4, '0')}.bin`);
    const fileBytes = Math.min(remaining, maxFileBytes);
    const file = await open(path, 'w');
    try {
      let unwritten = fileBytes;
      while (unwritten > 0) {
        const chunkBytes = Math.min(unwritten, bufferBytes);
        const chunk = randomBytes(chunkBytes);
        let offset = 0;
        while (offset < chunk.length) {
          const { bytesWritten } = await file.write(chunk, offset);
          offset += bytesWritten;
        }
        unwritten -= chunkBytes;
      }
    } finally {
      await file.close();
    }
    paths.push(path);
    remaining -= fileBytes;
  }
  return paths;
}

export async function writeLandingFile(directory, bytes, update) {
  const marker = Buffer.from(`<p>Scope load-test README update ${update}</p>\n`);
  const content = Buffer.alloc(bytes, 'x');
  marker.copy(content, 0, 0, Math.min(marker.length, content.length));
  await writeFile(join(directory, 'README.html'), content);
}
