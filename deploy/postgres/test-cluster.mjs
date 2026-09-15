// Tests that need a throwaway PostgreSQL cluster run initdb themselves, which
// refuses to run as root. Skip with a reason wherever that cannot work, such as
// the Scope checks container, instead of failing on a missing binary.
import { existsSync } from 'node:fs';
import { join } from 'node:path';

export const pgBin = process.env.SCOPE_TEST_POSTGRES_BIN ?? '/usr/lib/postgresql/16/bin';

export const localClusterSkip = !existsSync(join(pgBin, 'initdb'))
  ? `no initdb under ${pgBin}; set SCOPE_TEST_POSTGRES_BIN`
  : process.getuid?.() === 0
    ? 'initdb cannot run as root'
    : false;
