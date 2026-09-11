import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "dev/local-process.py"


class LocalProcessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="scope-local-process-")
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)

    def helper(self, *args):
        return subprocess.run(["python3", str(HELPER), *map(str, args)], check=True,
                              capture_output=True, text=True).stdout.strip()

    def child(self, script="sleep 60"):
        child = subprocess.Popen(["bash", "-c", script], start_new_session=True)
        def cleanup():
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=5)
        self.addCleanup(cleanup)
        return child

    def test_reused_pid_identity_and_obsolete_numeric_record_never_signal(self):
        child = self.child()
        path = self.path / "api.pid"
        self.helper("record", path, child.pid)
        original = json.loads(path.read_text())
        self.assertEqual(self.helper("status", path), f"running pid {child.pid}")
        for bad in [str(child.pid), {**original, "start": "0"}, {**original, "boot": "other-boot"}]:
            path.write_text(json.dumps(bad) if isinstance(bad, dict) else bad)
            self.assertEqual(self.helper("status", path), "stale")
            self.helper("stop", path)
            self.assertIsNone(child.poll())
            self.assertFalse(path.exists())

    def test_owned_session_stops_descendants_that_ignore_term(self):
        descendant = self.path / "descendant"
        child = self.child(f"trap '' TERM; sleep 60 & echo $! > '{descendant}'; wait")
        path = self.path / "api.pid"
        self.helper("record", path, child.pid)
        for _ in range(100):
            if descendant.exists():
                break
            time.sleep(0.01)
        pid = int(descendant.read_text())
        self.helper("stop", path)
        child.wait(timeout=5)
        for _ in range(100):
            stat = Path(f"/proc/{pid}/stat")
            if not stat.exists() or stat.read_text().rsplit(")", 1)[1].split()[0] == "Z":
                break
            time.sleep(0.01)
        else:
            self.fail("owned descendant survived stop")
        self.assertFalse(path.exists())

    def checkout(self):
        root = self.path / "repo"
        (root / "dev").mkdir(parents=True)
        shutil.copy(ROOT / "dev/scope-dev", root / "dev/scope-dev")
        shutil.copy(HELPER, root / "dev/local-process.py")
        return root

    def reset(self, root):
        return subprocess.run(["bash", str(root / "dev/scope-dev"), "reset"], capture_output=True,
                              text=True, env={**os.environ, "SCOPE_DEV_PUBLIC_HOST": "localhost"})

    def test_reset_checks_both_paths_before_any_removal(self):
        for symlink_parent, target in [(".tmp", "local-dev"), (".scope", "dev")]:
            with self.subTest(parent=symlink_parent):
                root = self.checkout()
                external = self.path / "external"
                (external / target).mkdir(parents=True)
                sentinel = external / target / "sentinel"
                sentinel.write_text("keep")
                (root / symlink_parent).symlink_to(external, target_is_directory=True)
                other = root / (".scope/dev" if symlink_parent == ".tmp" else ".tmp/local-dev")
                other.mkdir(parents=True)
                (other / "sentinel").write_text("keep")
                result = self.reset(root)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("refusing to remove data outside repo", result.stderr)
                self.assertEqual(sentinel.read_text(), "keep")
                self.assertTrue((other / "sentinel").exists())
                shutil.rmtree(root)
                shutil.rmtree(external)

    def test_reset_removes_normal_local_state(self):
        root = self.checkout()
        for name in [".tmp/local-dev", ".scope/dev"]:
            (root / name).mkdir(parents=True)
            (root / name / "sentinel").write_text("remove")
        result = self.reset(root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((root / ".tmp/local-dev").exists())
        self.assertFalse((root / ".scope/dev").exists())


if __name__ == "__main__":
    unittest.main()
