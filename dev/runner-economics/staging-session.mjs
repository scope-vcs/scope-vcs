// Local-only fixture setup. Never resets the catalog or authenticates a real user.
import { spawnSync } from 'node:child_process';
import { randomBytes, createHash } from 'node:crypto';
import { statSync, writeFileSync, unlinkSync } from 'node:fs';
import { resolve } from 'node:path';

const directory = resolve(process.argv[2]);
if (!directory.startsWith('/tmp/scope-bench-session.') || (statSync(directory).mode & 0o077)) {
  throw new Error('An owner-only experiment session directory is required');
}
const env = {
  ...process.env,
  RAILWAY_CALLER: 'skill:use-railway@1.4.0',
  RAILWAY_AGENT_SESSION: 'scope-runner-economics-20260907',
  SCOPE_API_URL: 'https://scope-api-staging.up.railway.app',
  XDG_CONFIG_HOME: directory,
};
const variables = spawnSync('railway', [
  'variable', 'list', '--json',
  '--project', '45dd67fa-6d69-48ad-9680-1313d41b4490',
  '--environment', '8743c2e9-5d9b-4161-a047-9617e6d199b9',
  '--service', '44c41a90-2d1a-4a1b-85c9-6af85d179be3',
], { env, encoding: 'utf8' });
if (variables.status !== 0) throw new Error('Cannot read staging database configuration');
const url = new URL(JSON.parse(variables.stdout).DATABASE_PUBLIC_URL);
const token = `scope_otc_${randomBytes(32).toString('hex')}`;
const hash = `sha256:${createHash('sha256').update(token).digest('hex')}`;
const now = Math.floor(Date.now() / 1000);
const result = spawnSync('psql', ['-X', '-v', 'ON_ERROR_STOP=1', '-At'], {
  env: {
    ...env, PGHOST: url.hostname, PGPORT: url.port,
    PGUSER: decodeURIComponent(url.username), PGPASSWORD: decodeURIComponent(url.password),
    PGDATABASE: url.pathname.slice(1), PGSSLMODE: 'require', PGCONNECT_TIMEOUT: '15',
  },
  // The grant uses the existing five-minute exchange format. The public API
  // consumes it through its normal login path and owns creation of the session.
  input: `INSERT INTO scope_cli_exchange_grants
    (grant_hash, user_id, created_at_unix, expires_at_unix, consumed_at_unix)
    SELECT '${hash}', id, ${now}, ${now + 300}, NULL FROM scope_users
    WHERE id = 'scope_usr_dev_seed' AND handle = 'dev' RETURNING user_id;`,
  encoding: 'utf8',
});
if (result.status !== 0 || !result.stdout.split('\n').includes('scope_usr_dev_seed')) {
  throw new Error('Cannot create the staging fixture exchange grant');
}
const path = `${directory}/exchange-token`;
writeFileSync(path, token, { mode: 0o600, flag: 'wx' });
try {
  const login = spawnSync(process.env.SCOPE_CLI_BINARY ?? 'scope', [
    'login', '--exchange-file', path, '--json',
  ], { env, stdio: 'inherit' });
  process.exitCode = login.status ?? 1;
} finally {
  unlinkSync(path);
}
