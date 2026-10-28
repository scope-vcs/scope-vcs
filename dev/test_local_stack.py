import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "dev/local-process.py"
STACK = """
set -euo pipefail
STATE_DIR="$ROOT/.tmp/local-dev"
WITH_MEDIA="${WITH_MEDIA:-1}"
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
tailnet_host() {
  echo "${TAILNET_HOST:-}"
}
tailscale() {
  echo "$ROOT: tailscale $*" >> "$TAILSCALE_LOG"
}
local_database_url() {
  echo "${TEST_DATABASE_URL:-postgres://scope@127.0.0.1/scope_dev}"
}
run_on_configured_database() {
  [[ -z "${DATABASE_FAILS:-}" ]] || return 1
  echo "$ROOT: $1" >> "$DATABASE_LOG"
}
use_slot "$(recorded_slot)"
case "$ACTION" in
  claim)
    claim_isolated_slot
    echo "$SLOT $WEB_PORT $API_PORT"
    ;;
  recreate) with_stacks_lock recreate_owned_database ;;
  failing-startup)
    trap release_isolated_stack EXIT
    with_stacks_lock recreate_owned_database
    ;;
  slot) echo "$SLOT" ;;
  origin) slot_tailnet_origin "$WEB_PORT" ;;
  database) isolated_database url ;;
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
        self.tailscale_log = self.path / "tailscale.log"

    def worktree(self, name):
        root = self.path / name
        (root / "dev").mkdir(parents=True)
        (root / "dev/local-process.py").symlink_to(HELPER)
        self.addCleanup(self.stop_expiry, root)
        return root

    def stop_expiry(self, root):
        subprocess.run(["python3", str(HELPER), "stop", str(root / ".tmp/local-dev/expiry.pid")],
                       check=True, capture_output=True)

    def run_stack(self, action, root, busy_ports="", **env):
        return subprocess.run(
            ["bash", "-c", STACK], capture_output=True, text=True, timeout=30,
            env={"PATH": "/usr/bin:/bin", "ROOT": str(root), "ACTION": action, "BUSY_PORTS": busy_ports,
                 "DATABASE_LOG": str(self.database_log), "TAILSCALE_LOG": str(self.tailscale_log),
                 "XDG_RUNTIME_DIR": str(self.runtime),
                 "STACK_SCRIPT": str(ROOT / "dev/local-stack.sh"), **env})

    def claim(self, root, busy_ports="", **env):
        return self.run_stack("claim", root, busy_ports, **env)

    def release(self, root):
        result = self.run_stack("release", root)
        self.assertEqual(result.returncode, 0, result.stderr)

    def database_commands(self):
        return self.database_log.read_text().splitlines() if self.database_log.exists() else []

    def claimed(self, root, busy_ports="", **env):
        result = self.claim(root, busy_ports, **env)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_claim_skips_a_slot_whose_ports_are_taken(self):
        first = self.worktree("first")

        self.assertEqual(self.claimed(first, busy_ports="8090"), "2 3020 8100")
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-2").read_text().strip(), str(first))
        self.assertEqual((first / ".tmp/local-dev/slot").read_text().strip(), "2")

    def test_claim_ignores_ports_of_services_the_stack_will_not_start(self):
        root = self.worktree("no-media")
        media_ports = "8093 8094 8103 8104"

        self.assertEqual(self.claim(root, busy_ports=media_ports).returncode, 1)
        self.assertEqual(self.claimed(root, busy_ports=media_ports, WITH_MEDIA="0"), "1 3010 8090")

    def test_isolated_database_replaces_the_name_and_refuses_target_overrides(self):
        root = self.worktree("database")
        self.claimed(root)

        def database(url):
            return self.run_stack("database", root, TEST_DATABASE_URL=url)

        allowed = database("postgres://scope:secret@localhost:5432/scope_dev?sslmode=disable")
        self.assertEqual(allowed.stdout.strip(),
                         "postgres://scope:secret@localhost:5432/scope_dev_stack1?sslmode=disable")
        for url in ["postgres://localhost/scope_dev?host=db.example.net",
                    "postgres://localhost/scope_dev?hostaddr=10.0.0.5",
                    "postgres://localhost/scope_dev?dbname=scope_test",
                    "postgres://localhost/scope_dev?service=remote",
                    "postgres://db.example.net/scope_dev",
                    "postgres://localhost/scope_dev_" + "x" * 47]:
            result = database(url)
            self.assertNotEqual(result.returncode, 0, url)
            self.assertEqual(result.stdout, "", url)

    def test_a_reclaimed_slot_no_longer_targets_the_stale_checkout(self):
        stale, replacement = self.worktree("stale"), self.worktree("replacement")
        self.claimed(stale)
        self.stop_expiry(stale)
        self.claimed(replacement)

        self.assertEqual(self.run_stack("slot", stale).stdout.strip(), "0")
        self.assertEqual(self.run_stack("slot", replacement).stdout.strip(), "1")

    def test_live_claims_are_kept_and_a_claim_without_its_expiry_is_reclaimed(self):
        first, second, third = self.worktree("first"), self.worktree("second"), self.worktree("third")

        self.assertEqual(self.claimed(first), "1 3010 8090")
        self.assertEqual(self.claimed(second), "2 3020 8100")
        self.stop_expiry(first)

        self.assertEqual(self.claimed(third), "1 3010 8090")
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-1").read_text().strip(), str(third))

    def tailscale_commands(self):
        return self.tailscale_log.read_text().splitlines() if self.tailscale_log.exists() else []

    def test_a_stale_owner_release_leaves_the_replacement_claim_database_and_tailnet_ports(self):
        stale, replacement = self.worktree("stale"), self.worktree("replacement")
        self.claimed(stale, TAILNET_HOST="host.tailnet.ts.net")
        self.stop_expiry(stale)
        self.claimed(replacement, TAILNET_HOST="host.tailnet.ts.net")
        self.assertEqual(self.run_stack("origin", replacement).stdout.strip(), "https://host.tailnet.ts.net:43010")
        self.tailscale_log.unlink()

        self.release(stale)

        self.assertEqual(self.database_commands(), [])
        self.assertEqual(self.tailscale_commands(), [])
        self.assertEqual((self.runtime / "scope-dev-stacks/slot-1").read_text().strip(), str(replacement))
        self.assertFalse((stale / ".tmp/local-dev/slot").exists())

        self.release(replacement)

        self.assertEqual(self.database_commands(),
                         [f'{replacement}: DROP DATABASE IF EXISTS "scope_dev_stack1" WITH (FORCE)'])
        self.assertEqual(self.tailscale_commands(),
                         [f"{replacement}: tailscale serve --https={port} off" for port in (43010, 48090, 48093)])
        self.assertEqual(self.run_stack("origin", replacement).stdout.strip(), "")
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

    def test_a_failed_startup_releases_its_claim_without_deadlocking_the_slot_lock(self):
        root = self.worktree("failing")
        self.claimed(root)

        result = self.run_stack("failing-startup", root, DATABASE_FAILS="1")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not drop", result.stderr)
        self.assertFalse((self.runtime / "scope-dev-stacks/slot-1").exists())
        subprocess.run(["flock", "-n", str(self.runtime / "scope-dev-stacks/lock"), "true"], check=True, timeout=5)

    def test_a_claim_stays_live_while_its_checkout_holds_its_lock(self):
        busy, other = self.worktree("busy"), self.worktree("other")
        (busy / "dev/scope-dev").write_text("")
        self.claimed(busy)
        self.stop_expiry(busy)
        with open(busy / "dev/scope-dev") as checkout_lock:
            fcntl.flock(checkout_lock, fcntl.LOCK_EX)

            self.assertEqual(self.claimed(other), "2 3020 8100")

        third = self.worktree("third")
        self.assertEqual(self.claimed(third), "1 3010 8090")

    def test_expiry_runs_down_for_its_own_claim(self):
        root = self.worktree("expiring")
        down_marker = root / "down-claim"
        (root / "dev/scope-dev").write_text(
            f'#!/usr/bin/env bash\necho "$SCOPE_DEV_EXPIRING_CLAIM $1" > "{down_marker}"\n')
        (root / "dev/scope-dev").chmod(0o755)
        self.claimed(root)
        expiry = (root / ".tmp/local-dev/expiry.sh").read_text().replace("sleep 3600", "sleep 0")

        subprocess.run(["bash", "-c", expiry], check=True, timeout=30)

        self.wait_for(lambda: down_marker.exists() and down_marker.read_text().strip())
        claim = (root / ".tmp/local-dev/claim").read_text().strip()
        self.assertEqual(down_marker.read_text().split(), [claim, "down"])

    def test_down_for_a_replaced_claim_leaves_the_running_stack(self):
        root = self.worktree("replaced")
        for name in ["scope-dev", "dotenv.sh", "local-stack.sh"]:
            shutil.copy(ROOT / "dev" / name, root / "dev" / name)
        self.claimed(root)
        expiry_pid = root / ".tmp/local-dev/expiry.pid"

        result = subprocess.run(
            ["bash", str(root / "dev/scope-dev"), "down"], capture_output=True, text=True, timeout=30,
            env={**os.environ, "SCOPE_DEV_EXPIRING_CLAIM": "an-earlier-claim", "XDG_RUNTIME_DIR": str(self.runtime)})

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already replaced", result.stdout)
        status = subprocess.run(["python3", str(HELPER), "status", str(expiry_pid)],
                                capture_output=True, text=True, check=True).stdout
        self.assertTrue(status.startswith("running"), status)
        self.assertTrue((self.runtime / "scope-dev-stacks/slot-1").exists())

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
