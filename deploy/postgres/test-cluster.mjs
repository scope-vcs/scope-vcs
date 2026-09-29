// Tests that need a throwaway PostgreSQL cluster run initdb themselves, which
// refuses to run as root. Skip with a reason wherever that cannot work, such as
// the Scope checks container, instead of failing on a missing binary. CI sets
// SCOPE_REQUIRE_POSTGRES_CLUSTER=1 so that the same condition fails instead.
import { existsSync } from 'node:fs';
import { join } from 'node:path';

export const pgBin = process.env.SCOPE_TEST_POSTGRES_BIN ?? '/usr/lib/postgresql/18/bin';

export const localClusterSkip = !existsSync(join(pgBin, 'initdb'))
  ? `no initdb under ${pgBin}; set SCOPE_TEST_POSTGRES_BIN`
  : process.getuid?.() === 0
    ? 'initdb cannot run as root'
    : false;

if (localClusterSkip && process.env.SCOPE_REQUIRE_POSTGRES_CLUSTER === '1') {
  throw new Error(`SCOPE_REQUIRE_POSTGRES_CLUSTER=1 but PostgreSQL cluster tests cannot run: ${localClusterSkip}`);
}
