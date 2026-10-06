#!/usr/bin/env node

import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const PREVIEW_LABEL = 'preview';
const SHA = /^[0-9a-f]{40}$/;

export async function verifyPreviewBuild({ run, repository, sourceSha, request }) {
  if (run?.event !== 'pull_request' || run.conclusion !== 'success') return { deploy: false, reason: 'build did not succeed' };
  if (run.head_repository?.full_name !== repository || run.pull_requests?.length !== 1) {
    return { deploy: false, reason: 'build is not for exactly one same-repository pull request' };
  }
  const number = run.pull_requests[0].number;
  if (!Number.isSafeInteger(number) || number <= 0 || !SHA.test(run.head_sha ?? '') || !SHA.test(sourceSha ?? '')) {
    throw new Error('Preview build identity is malformed.');
  }
  const pull = await request(`/repos/${repository}/pulls/${number}`);
  if (pull.state !== 'open' || pull.base?.ref !== 'main' || pull.head?.repo?.full_name !== repository ||
      !pull.labels?.some(({ name }) => name === PREVIEW_LABEL)) {
    return { deploy: false, reason: `pull request #${number} is not an open, labeled same-repository pull request into main` };
  }
  if (pull.head.sha !== run.head_sha) return { deploy: false, reason: `pull request #${number} has a newer head` };
  const merge = await request(`/repos/${repository}/commits/${sourceSha}`);
  const [base, head] = merge.parents?.map(({ sha }) => sha) ?? [];
  if (merge.parents?.length !== 2 || head !== run.head_sha) {
    throw new Error(`Prepared revision ${sourceSha} is not the merge of pull request #${number}.`);
  }
  const comparison = await request(`/repos/${repository}/compare/${base}...main`);
  if (!['ahead', 'identical'].includes(comparison.status)) throw new Error(`Prepared revision ${sourceSha} does not merge into main.`);
  return { deploy: true, pullRequest: number, sourceSha };
}

async function githubRequest(path) {
  const response = await fetch(`https://api.github.com${path}`, {
    headers: { Authorization: `Bearer ${process.env.GITHUB_TOKEN}`, Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) throw new Error(`GitHub request failed with ${response.status}.`);
  return response.json();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [preparedPath] = process.argv.slice(2);
    const { workflow_run: run } = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    const { sourceSha } = JSON.parse(readFileSync(preparedPath, 'utf8'));
    const result = await verifyPreviewBuild({ run, repository: process.env.GITHUB_REPOSITORY, sourceSha, request: githubRequest });
    console.log(result.deploy ? `Deploying ${sourceSha} for pull request #${result.pullRequest}.` : `Skipping preview: ${result.reason}.`);
    appendFileSync(process.env.GITHUB_OUTPUT, `deploy=${result.deploy}\npull_request=${result.pullRequest ?? ''}\nsource_sha=${result.sourceSha ?? ''}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
