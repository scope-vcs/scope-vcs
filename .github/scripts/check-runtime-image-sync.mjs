#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { readScopeManagedFile } from './scope-managed-files.mjs';

const read = (path) => readFileSync(path, 'utf8');
const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const targets = JSON.parse(read('cli/distribution/targets.json'));
const nodeVersion = targets.node_version;
const linuxNodeSha = targets.targets.find((target) => target.node_platform === 'linux-x64')?.node_sha256;
assert.match(nodeVersion, /^\d+\.\d+\.\d+$/);
assert.match(linuxNodeSha, /^[0-9a-f]{64}$/);

const checks = readScopeManagedFile('.scope/images/checks/Dockerfile');
const worker = read('deploy/railway/worker.Dockerfile');
const web = read('deploy/railway/web.Dockerfile');
if (checks !== undefined) {
  assert.match(checks, new RegExp(`ARG NODE_VERSION=${escape(nodeVersion)}\\b`));
  assert.match(checks, new RegExp(`ARG NODE_SHA256=${linuxNodeSha}\\b`));
}
assert.match(worker, new RegExp(`FROM node:${escape(nodeVersion)}-bookworm-slim@sha256:[0-9a-f]{64}`));
assert.match(worker, new RegExp(`ENV NODE_VERSION=${escape(nodeVersion)}\\b`));
assert.match(web, new RegExp(`FROM node:${escape(nodeVersion)}-bookworm-slim@sha256:[0-9a-f]{64}`));

const maintenance = read('deploy/railway/maintenance.Dockerfile');
const postgres = maintenance.match(/FROM postgres:(\d+\.\d+)@(sha256:[0-9a-f]{64})/);
assert.ok(postgres, 'maintenance image must pin PostgreSQL and its digest');
if (checks !== undefined) assert.match(checks, new RegExp(`ARG POSTGRES_VERSION=${escape(postgres[1])}-`));
for (const workflow of ['rust-workspace-checks', 'scope-integration-ci']) {
  assert.ok(read(`.github/workflows/${workflow}.yml`).includes(`postgres:${postgres[1]}@${postgres[2]}`));
}
assert.equal(
  read('.github/workflows/deploy-staging.yml').split(`postgres:${postgres[1]}@${postgres[2]}`).length - 1,
  2,
);

const crossWorkflow = read('.github/workflows/scope-cli-build.yml');
const crossVersion = crossWorkflow.match(/cargo install cross --version (\d+\.\d+\.\d+) --locked/)?.[1];
assert.ok(crossVersion, 'cross release must be exact');
assert.match(read('Cross.toml'), new RegExp(`image = "ghcr\\.io/cross-rs/aarch64-unknown-linux-gnu:${escape(crossVersion)}@sha256:[0-9a-f]{64}"`));

console.log(`Runtime images match Node ${nodeVersion}, PostgreSQL ${postgres[1]}, and cross ${crossVersion}.`);
