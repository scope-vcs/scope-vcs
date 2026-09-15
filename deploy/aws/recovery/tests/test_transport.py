"""Private collection rejects ambiguous identities and unrequested environment data."""
import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("recovery_run", ROOT / ".github/scripts/recovery-run.py")
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)
IDENTITY = "12345678-1234-1234-1234-123456789abc"


class TransportTests(unittest.TestCase):
    def testRejectsAmbiguousTarget(self):
        with self.assertRaises(transport.Incomplete):
            transport.Railway("production", IDENTITY, Path("identity"))
        railway = transport.Railway(IDENTITY, IDENTITY, Path("identity"))
        with self.assertRaises(transport.Incomplete):
            railway.run("maintenance", ["true"])

    def testPinsRemoteIdentityBeforeExecutingAndDropsKeyEnvironment(self):
        railway = transport.Railway(IDENTITY, IDENTITY, Path("identity"))
        with patch.dict(transport.os.environ, {"SCOPE_RAILWAY_SSH_PRIVATE_KEY": "fixture"}), patch.object(transport.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"result")) as run:
            self.assertEqual(railway.run(IDENTITY, ["bash", "-s"], b"true"), b"result")
        command = run.call_args.args[0]
        self.assertIn('--identity-file', command)
        for name in ("RAILWAY_PROJECT_ID", "RAILWAY_ENVIRONMENT_ID", "RAILWAY_SERVICE_ID"):
            self.assertIn(name, command[-1])
        self.assertIn("|| exit 2; exec bash -s", command[-1])
        self.assertNotIn("SCOPE_RAILWAY_SSH_PRIVATE_KEY", run.call_args.kwargs["env"])

    def testCollectorRejectsUnexpectedRuntimeFields(self):
        railway = transport.Railway(IDENTITY, IDENTITY, Path("identity"))
        with patch.object(railway, "run", return_value=b"UNEXPECTED=fixture\0"):
            with self.assertRaises(transport.Incomplete):
                railway.collect(IDENTITY, {"EXPECTED"})

    def testCollectorPreservesValuesWithoutPrintingThem(self):
        railway = transport.Railway(IDENTITY, IDENTITY, Path("identity"))
        with patch.object(railway, "run", return_value=b"EXPECTED=a=b\nsecond-line\0"):
            self.assertEqual(railway.collect(IDENTITY, {"EXPECTED"}), {"EXPECTED": "a=b\nsecond-line"})
