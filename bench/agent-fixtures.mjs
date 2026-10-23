import { createHash, randomBytes } from 'node:crypto';
import { open } from 'node:fs/promises';

const MAX_DIRECTORY_ENTRIES = 1000;
const AUTHOR = 'Scope Agent <agent@scope.local>';
const FIRST_COMMIT_SECONDS = 1_700_000_000;

export function slug(value, maxLength = 60) {
  return value.toLowerCase().replace(/[^a-z0-9-]/g, '-').slice(0, maxLength);
}

export function agentBranchName(label, index) {
  const labelHash = createHash('sha256').update(label).digest('hex').slice(0, 8);
  const suffix = `-${labelHash}-${index}`;
  return slug(`agent-${label}`, 60 - suffix.length) + suffix;
}

export function seedFilePath(index, fileCount) {
  let levels = 1;
  for (let capacity = MAX_DIRECTORY_ENTRIES; capacity < fileCount; capacity *= MAX_DIRECTORY_ENTRIES) levels += 1;
  const segments = [];
  let remainder = index;
  for (let level = 0; level < levels; level += 1) {
    segments.unshift(String(remainder % MAX_DIRECTORY_ENTRIES).padStart(3, '0'));
    remainder = Math.floor(remainder / MAX_DIRECTORY_ENTRIES);
  }
  return `seed/${segments.join('/')}.bin`;
}

export async function writeSeedStream(path, fileCount, fileBytes) {
  await writeStream(path, async (write) => {
    await write(commitHeader('refs/heads/main', 'Seed', FIRST_COMMIT_SECONDS));
    for (let index = 0; index < fileCount; index += 1) {
      await write(inlineFile(seedFilePath(index, fileCount), randomBytes(fileBytes)));
    }
  });
}

export async function writeAgentHistoryStream(path, { name, baseOid, commits, deltaBytes }) {
  await writeStream(path, async (write) => {
    for (let commit = 0; commit < commits; commit += 1) {
      await write(commitHeader('refs/heads/work', `${name} change ${commit}`, FIRST_COMMIT_SECONDS + commit + 1, commit + 1));
      if (commit === 0) await write(`from ${baseOid}\n`);
      await write(inlineFile(`agents/${name}/f-${commit}.bin`, randomBytes(deltaBytes)));
    }
  });
}

export function parseMarks(text, count) {
  const oids = new Map(text.split('\n').filter(Boolean).map((line) => {
    const [mark, oid] = line.split(' ');
    return [Number(mark.slice(1)), oid];
  }));
  return Array.from({ length: count }, (_, index) => {
    const oid = oids.get(index + 1);
    if (!oid) throw new Error(`fast-import did not export mark ${index + 1}`);
    return oid;
  });
}

function commitHeader(ref, message, seconds, mark = null) {
  const identity = `${AUTHOR} ${seconds} +0000`;
  return `commit ${ref}\n${mark === null ? '' : `mark :${mark}\n`}author ${identity}\ncommitter ${identity}\ndata ${Buffer.byteLength(message)}\n${message}\n`;
}

function inlineFile(path, content) {
  return Buffer.concat([
    Buffer.from(`M 100644 inline ${path}\ndata ${content.length}\n`),
    content,
    Buffer.from('\n'),
  ]);
}

async function writeStream(path, produce) {
  const handle = await open(path, 'w');
  try {
    await produce((chunk) => handle.write(chunk));
    await handle.write('done\n');
  } finally {
    await handle.close();
  }
}
