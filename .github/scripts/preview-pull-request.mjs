#!/usr/bin/env node

import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const PREVIEW_LABEL = 'preview';
const SHA = /^[0-9a-f]{40}$/;
const PULL_REQUEST = /^[1-9][0-9]{0,8}$/;

export function qualifiesForPreview(pull, repository) {
  return pull.state === 'open' && pull.base?.ref === 'main' && pull.head?.repo?.full_name === repository &&
    pull.labels?.some(({ name }) => name === PREVIEW_LABEL) === true;
}

export async function buildAction({ run, repository, sourceSha, request }) {
  if (run?.event !== 'pull_request' || run.conclusion !== 'success' || !sourceSha) {
    return { action: 'skip', reason: 'the run built no preview images' };
  }
  if (run.head_repository?.full_name !== repository || run.pull_requests?.length !== 1) {
    return { action: 'skip', reason: 'the build is not for exactly one same-repository pull request' };
  }
  const number = run.pull_requests[0].number;
  if (!Number.isSafeInteger(number) || number <= 0 || !SHA.test(run.head_sha ?? '') || !SHA.test(sourceSha)) {
    throw new Error('Preview build identity is malformed.');
  }
  const pull = await request(`/repos/${repository}/pulls/${number}`);
  if (!qualifiesForPreview(pull, repository)) {
    return { action: 'delete', pullRequest: number, reason: `pull request #${number} no longer qualifies for a preview` };
  }
  if (pull.head.sha !== run.head_sha) {
    return { action: 'skip', reason: `pull request #${number} has a newer head` };
  }
  const merge = await request(`/repos/${repository}/commits/${sourceSha}`);
  const [base, head] = merge.parents?.map(({ sha }) => sha) ?? [];
  if (merge.parents?.length !== 2 || head !== run.head_sha) {
    throw new Error(`Prepared revision ${sourceSha} is not the merge of pull request #${number}.`);
  }
  const comparison = await request(`/repos/${repository}/compare/${base}...main`);
  if (!['ahead', 'identical'].includes(comparison.status)) throw new Error(`Prepared revision ${sourceSha} does not merge into main.`);
  return { action: 'deploy', pullRequest: number, sourceSha };
}

export async function cleanupAction({ pullRequest, repository, request }) {
  if (!PULL_REQUEST.test(String(pullRequest))) throw new Error('A positive pull request number is required.');
  const pull = await request(`/repos/${repository}/pulls/${pullRequest}`);
  return qualifiesForPreview(pull, repository)
    ? { action: 'skip', reason: `pull request #${pullRequest} qualifies for a preview again` }
    : { action: 'delete', pullRequest: Number(pullRequest) };
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
    const [command, target] = process.argv.slice(2);
    const repository = process.env.GITHUB_REPOSITORY;
    let result;
    if (command === 'build') {
      const { workflow_run: run } = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      const sourceSha = target ? JSON.parse(readFileSync(target, 'utf8')).sourceSha : '';
      result = await buildAction({ run, repository, sourceSha, request: githubRequest });
    } else if (command === 'cleanup') {
      result = await cleanupAction({ pullRequest: target, repository, request: githubRequest });
    } else {
      throw new Error('usage: preview-pull-request.mjs build [prepared-release.json] | cleanup <pull-request-number>');
    }
    console.log(result.reason ? `${result.action}: ${result.reason}.` : `${result.action}: pull request #${result.pullRequest}.`);
    appendFileSync(process.env.GITHUB_OUTPUT,
      `action=${result.action}\npull_request=${result.pullRequest ?? ''}\nsource_sha=${result.sourceSha ?? ''}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
