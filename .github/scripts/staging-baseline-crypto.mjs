import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';
import { pathToFileURL } from 'node:url';

const version = Buffer.from('SCOPE-STAGING-BASELINE\x01');
const nonceLength = 12;
const tagLength = 16;
const headerLength = version.length + nonceLength;

export function snapshotKey(value) {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/i.test(value)) {
    throw new Error('SCOPE_STAGING_BASELINE_KEY must contain exactly 64 hexadecimal characters.');
  }
  return Buffer.from(value, 'hex');
}

// Metadata is authenticated with the dump: its environment, ledger and restore
// policy cannot be changed independently of the encrypted database snapshot.
export async function transformSnapshot(action, inputPath, outputPath, metadataPath, keyValue) {
  const key = snapshotKey(keyValue);
  if (!['encrypt', 'decrypt'].includes(action)) throw new Error('Unknown baseline encryption action.');
  if (resolve(inputPath) === resolve(outputPath)) throw new Error('Snapshot input and output must differ.');
  const metadata = await readFile(metadataPath);
  const temporary = join(dirname(outputPath), `.baseline-${randomBytes(16).toString('hex')}.tmp`);
  try {
    if (action === 'encrypt') {
      if ((await stat(inputPath)).size === 0) throw new Error('Cannot encrypt an empty database snapshot.');
      const nonce = randomBytes(nonceLength);
      const header = Buffer.concat([version, nonce]);
      const cipher = createCipheriv('aes-256-gcm', key, nonce);
      cipher.setAAD(Buffer.concat([header, metadata]));
      await writeFile(temporary, header, { flag: 'wx', mode: 0o600 });
      await pipeline(createReadStream(inputPath), cipher, createWriteStream(temporary, { flags: 'a' }));
      await writeFile(temporary, cipher.getAuthTag(), { flag: 'a' });
    } else {
      const file = await open(inputPath, 'r');
      let header, tag, size;
      try {
        size = (await file.stat()).size;
        if (size <= headerLength + tagLength) throw new Error('Invalid encrypted baseline envelope.');
        header = Buffer.alloc(headerLength);
        tag = Buffer.alloc(tagLength);
        await file.read(header, 0, header.length, 0);
        await file.read(tag, 0, tag.length, size - tagLength);
      } finally {
        await file.close();
      }
      if (!header.subarray(0, version.length).equals(version)) throw new Error('Unsupported encrypted baseline version.');
      const decipher = createDecipheriv('aes-256-gcm', key, header.subarray(version.length));
      decipher.setAAD(Buffer.concat([header, metadata]));
      decipher.setAuthTag(tag);
      // Unauthenticated plaintext stays in a private temporary file. Only a
      // complete, authenticated result is published for pg_restore to consume.
      await pipeline(
        createReadStream(inputPath, { start: headerLength, end: size - tagLength - 1 }),
        decipher,
        createWriteStream(temporary, { flags: 'wx', mode: 0o600 }),
      );
    }
    await rename(temporary, outputPath);
  } finally {
    key.fill(0);
    await rm(temporary, { force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [action, inputPath, outputPath, metadataPath, extra] = process.argv.slice(2);
  if (!action || !inputPath || !outputPath || !metadataPath || extra) {
    console.error('usage: staging-baseline-crypto.mjs encrypt|decrypt <input> <output> <baseline.json>');
    process.exitCode = 1;
  } else {
    transformSnapshot(action, inputPath, outputPath, metadataPath, process.env.SCOPE_STAGING_BASELINE_KEY)
      .catch(error => { console.error(error.message); process.exitCode = 1; });
  }
}
