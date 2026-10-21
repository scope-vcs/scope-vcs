from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "dev/local-process.py"
STACK = """
set -euo pipefail
STATE_DIR="$ROOT/.tmp/local-dev"
port_owner() {
  if [[ " $BUSY_PORTS " == *" $1 "* ]]; then
    echo 1
  fi
}
start_service() {
  setsid sleep 60 > /dev/null 2>&1 &
  python3 "$ROOT/dev/local-process.py" record "$STATE_DIR/$1.pid" "$!"
}
source "$STACK_SCRIPT"
local_database_url() {
  echo postgres://scope@127.0.0.1/scope_dev
}
run_on_configured_database() {
  echo "$ROOT: $1" >> "$DATABASE_LOG"
}
use_slot "$(recorded_slot)"
case "$ACTION" in
  claim)
    claim_isolated_slot
    echo "$SLOT $WEB_PORT $API_PORT"
    ;;
  recreate) with_stacks_lock recreate_owned_database ;;
  release) release_isolated_stack ;;
esac
"""


class LocalStackClaimTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="scope-local-stack-")
        self.addCleanup(temp.cleanup)
        self.path = Path(temp.name)
        self.runtime = self.path / "runtime"
        self.database_log = self.path / "database.log"

    def worktree(self, name):
        root = self.path / name
        (root / "dev").mkdir(parents=True)
        (root / "dev/local-process.py").symlink_to(HELPER)
        self.addCleanup(self.stop_expiry, root)
        return root

    def stop_expiry(self, root):
        subprocess.run(["python3", str(HELPER), "stop", str(root / ".tmp/local-dev/expiry.pid")],
                       check=True, capture_output=True)

    def run_stack(self, action, root, busy_ports=""):
        return subprocess.run(
            ["bash", "-c", STACK], capture_output=True, text=True, timeout=30,
            env={"PATH": "/usr/bin:/bin", "ROOT": str(root), "ACTION": action, "BUSY_PORTS": busy_ports,
                 "DATABASE_LOG": str(self.database_log), "XDG_RUNTIME_DIR": str(self.runtime),
                 "STACK_SCRIPT": str(ROOT / "dev/local-stack.sh")})

    def claim(self, root, busy_ports=""):
        return self.run_stack("claim", root, busy_ports)

    def release(self, root):
        result = self.run_stack("release", root)
        self.assertEqual(result.returncode, 0, result.stderr)

    def database_commands(self):
        return self.database_log.read_text().splitlines() if self.database_log.exists() else []

    def claimed(self, root, busy_ports=""):
        result = self.claim(root, busy_ports)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_claim_skips_a_slot_whose_ports_are_taken(self):
        first = self.worktree("first")

        self.assertEqual(self.claimed(first, busy_ports="8090"), "2 3020 8100")
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-2").read_text().strip(), str(first))
        self.assertEqual((first / ".tmp/local-dev/slot").read_text().strip(), "2")

    def test_live_claims_are_kept_and_a_claim_without_its_expiry_is_reclaimed(self):
        first, second, third = self.worktree("first"), self.worktree("second"), self.worktree("third")

        self.assertEqual(self.claimed(first), "1 3010 8090")
        self.assertEqual(self.claimed(second), "2 3020 8100")
        self.stop_expiry(first)

        self.assertEqual(self.claimed(third), "1 3010 8090")
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-1").read_text().strip(), str(third))

    def test_a_stale_owner_release_leaves_the_replacement_claim_and_database(self):
        stale, replacement = self.worktree("stale"), self.worktree("replacement")
        self.claimed(stale)
        self.stop_expiry(stale)
        self.claimed(replacement)

        self.release(stale)

        self.assertEqual(self.database_commands(), [])
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-1").read_text().strip(), str(replacement))
        self.assertFalse((stale / ".tmp/local-dev/slot").exists())

        self.release(replacement)

        self.assertEqual(self.database_commands(),
                         [f'{replacement}: DROP DATABASE IF EXISTS "scope_dev_stack1" WITH (FORCE)'])
        self.assertFalse((self.runtime / "scope-dev-stacks/slot-1").exists())

    def test_a_startup_that_outlived_its_claim_cannot_recreate_the_replacement_database(self):
        stale, replacement = self.worktree("stale"), self.worktree("replacement")
        self.claimed(stale)
        self.stop_expiry(stale)
        self.claimed(replacement)

        result = self.run_stack("recreate", stale)

        self.assertEqual(result.returncode, 1)
        self.assertIn("expired before startup finished", result.stderr)
        self.assertEqual(self.database_commands(), [])

    def test_expiry_runs_down_only_after_the_startup_that_claimed_the_slot_exits(self):
        root = self.worktree("starting")
        down_marker = root / "down-ran"
        (root / "dev/scope-dev").write_text(f'#!/usr/bin/env bash\ntouch "{down_marker}"\n')
        (root / "dev/scope-dev").chmod(0o755)
        expiry = root / ".tmp/local-dev/expiry.sh"
        startup = subprocess.Popen(
            ["bash", "-c", 'mkdir -p "$STATE_DIR"; source "$STACK_SCRIPT"; ISOLATED_LIFETIME_SECONDS=0; '
             'write_expiry_runner; read -r'],
            stdin=subprocess.PIPE,
            env={"PATH": "/usr/bin:/bin", "ROOT": str(root), "STATE_DIR": str(expiry.parent),
                 "STACK_SCRIPT": str(ROOT / "dev/local-stack.sh")})
        self.addCleanup(startup.kill)
        self.wait_for(expiry.exists)

        expiry_process = subprocess.Popen(["bash", str(expiry)])
        self.addCleanup(expiry_process.kill)
        time.sleep(1)
        self.assertFalse(down_marker.exists())

        startup.stdin.close()
        startup.wait(timeout=5)
        expiry_process.wait(timeout=30)
        self.wait_for(down_marker.exists)

    def wait_for(self, condition):
        deadline = time.monotonic() + 30
        while not condition():
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.1)

    def test_claim_names_the_owners_when_every_slot_is_in_use(self):
        first, second, third = self.worktree("first"), self.worktree("second"), self.worktree("third")
        self.claimed(first)
        self.claimed(second)

        result = self.claim(third)

        self.assertEqual(result.returncode, 1)
        self.assertIn(f"slot 1: {first}", result.stderr)
        self.assertIn(f"slot 2: {second}", result.stderr)
        self.assertFalse((third / ".tmp/local-dev/slot").exists())


if __name__ == "__main__":
    unittest.main()
