import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

test('Railway SSH keeps strict pinned trust even when later arguments request permissive settings', () => {
  const wrapper = fileURLToPath(new URL('../../deploy/railway/ssh-bin/ssh', import.meta.url));
  const pin = fileURLToPath(new URL('../../deploy/railway/ssh_known_hosts', import.meta.url));
  const output = execFileSync(wrapper, ['-G', '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null', 'ssh.railway.com'], {encoding:'utf8',stdio:['ignore','pipe','pipe']});
  assert.match(output, /^stricthostkeychecking true$/m);
  assert.ok(output.split('\n').includes(`userknownhostsfile ${pin}`));
  assert.match(output, /^globalknownhostsfile \/dev\/null$/m);
  assert.match(output, /^updatehostkeys false$/m);
  const keys = execFileSync('ssh-keygen', ['-F','ssh.railway.com','-f',pin], {encoding:'utf8'});
  assert.match(keys, /^ssh\.railway\.com ssh-ed25519 /m);
});
