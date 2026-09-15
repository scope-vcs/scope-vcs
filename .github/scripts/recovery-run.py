"""Daily GitHub entrypoint: scoped Railway SSH collection and OIDC-only AWS publication."""

import io
import json
import os
import re
import shlex
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/aws/recovery"))
import capture
from common import Incomplete, KEYS, PREFIXES, required, write_json
from snapshot import metadata


UUID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}")
SUFFIXES = ("ENDPOINT", "NAME", "REGION", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY", "FORCE_PATH_STYLE")


class Railway:
    def __init__(self, project, environment, identity):
        if not UUID.fullmatch(project) or not UUID.fullmatch(environment):
            raise Incomplete("recovery requires explicit Railway IDs")
        self.project, self.environment, self.identity = project, environment, identity

    def run(self, service, arguments, incoming=None, output=None):
        if not UUID.fullmatch(service):
            raise Incomplete("recovery requires an explicit service ID")
        # Remote identity checks precede every operation. All shell operands are
        # quoted and runtime secrets are never interpolated into command text.
        guard = f'test "$RAILWAY_PROJECT_ID" = {shlex.quote(self.project)} && test "$RAILWAY_ENVIRONMENT_ID" = {shlex.quote(self.environment)} && test "$RAILWAY_SERVICE_ID" = {shlex.quote(service)} || exit 2; exec '
        command = guard + shlex.join(arguments)
        environment = dict(os.environ, RAILWAY_CALLER="skill:use-railway@1.4.0", RAILWAY_AGENT_SESSION="scope-recovery-" + os.environ.get("GITHUB_RUN_ID", "local"))
        environment.pop("SCOPE_RAILWAY_SSH_PRIVATE_KEY", None)
        result = subprocess.run(["railway", "ssh", "--project", self.project, "--environment", self.environment, "--service", service, "--identity-file", str(self.identity), "--", command],
                                input=incoming, stdout=output if output is not None else subprocess.PIPE, stderr=subprocess.PIPE, env=environment, timeout=2400)
        if result.returncode:
            raise Incomplete("private Railway recovery collection failed")
        return result.stdout

    def collect(self, service, names):
        script = 'set -eu\nfor name in "$@"; do if [[ -v "$name" ]]; then printf "%s=%s\\0" "$name" "${!name}"; fi; done\n'
        raw = self.run(service, ["bash", "-s", "--", *sorted(names)], script.encode())
        values = {}
        for field in raw.split(b"\0"):
            if not field:
                continue
            name, separator, value = field.partition(b"=")
            decoded = name.decode()
            if not separator or decoded not in names:
                raise Incomplete("runtime environment response contained unexpected fields")
            values[decoded] = value.decode()
        return values

    def snapshot(self, service, destination, include_cache=False):
        query = (ROOT / "deploy/aws/recovery/references.sql").read_text().rstrip().rstrip(";")
        files = {
            "remote-snapshot.sh": (ROOT / "deploy/aws/recovery/remote-snapshot.sh").read_bytes(),
            "references-json.sql": ("SELECT row_to_json(recovery_row) FROM (" + query + ") recovery_row;\n").encode(),
        }
        package = io.BytesIO()
        with tarfile.open(fileobj=package, mode="w") as archive:
            for name, content in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
        wrapper = """
set -eu
umask 077
directory=$(mktemp -d /tmp/scope-recovery.XXXXXXXX)
trap 'rm -rf -- "$directory"' EXIT
trap "exit 1" HUP INT TERM
tar -xf - -C "$directory"
bash "$directory/remote-snapshot.sh" "$directory"
"""
        with tempfile.TemporaryFile() as transferred:
            self.run(service, ["bash", "-ceu", wrapper], package.getvalue(), transferred)
            transferred.seek(0)
            contents = {}
            with tarfile.open(fileobj=transferred, mode="r:gz") as archive:
                for member in archive:
                    if not member.isfile() or member.name not in ("database.dump", "references.jsonl", "captured-at.txt") or member.name in contents:
                        raise Incomplete("snapshot transfer contained an unexpected entry")
                    if member.size > (1024**3 if member.name == "database.dump" else 64 * 1024**2):
                        raise Incomplete("snapshot transfer exceeded its bounds")
                    if member.name == "database.dump":
                        with archive.extractfile(member) as source, Path(destination).open("xb") as target:
                            while chunk := source.read(1024**2):
                                target.write(chunk)
                        contents[member.name] = True
                    else:
                        contents[member.name] = archive.extractfile(member).read()
            if set(contents) != {"database.dump", "references.jsonl", "captured-at.txt"}:
                raise Incomplete("snapshot transfer is missing a component")
        rows = [json.loads(line) for line in contents["references.jsonl"].splitlines() if line]
        return metadata(rows, contents["captured-at.txt"].decode().strip(), destination, include_cache)


def main():
    os.umask(0o077)
    manifest = json.loads((ROOT / ".github/deployment-services.json").read_bytes())
    with tempfile.TemporaryDirectory(prefix="scope-recovery-credentials-") as scratch:
        root = Path(scratch)
        identity = root / "ssh-key"
        identity.write_text(required("SCOPE_RAILWAY_SSH_PRIVATE_KEY") + "\n")
        identity.chmod(0o600)
        railway = Railway(manifest["railway"]["projectId"], manifest["environments"]["production"]["environmentId"], identity)
        maintenance = manifest["railway"]["maintenanceServiceId"]
        object_names = {PREFIXES["objects"] + "_" + suffix for suffix in SUFFIXES} | {"SCOPE_OBJECT_ENCRYPTION_KEY"}
        media_names = {PREFIXES["media"] + "_" + suffix for suffix in SUFFIXES} | {"SCOPE_MEDIA_ENCRYPTION_KEY"}
        values = railway.collect(maintenance, object_names)
        values.update(railway.collect(manifest["services"]["media-api"]["id"], media_names))
        if os.environ.get("SCOPE_RECOVERY_INCLUDE_CACHE", "").lower() == "true":
            values.update(railway.collect(manifest["services"]["cache"]["id"], {PREFIXES["cache"] + "_" + suffix for suffix in SUFFIXES}))
        # Values remain in this process and mode-0600 scratch. Never write them to
        # GITHUB_ENV, step outputs, console logs, or uploaded workflow artifacts.
        write_json(root / "keys.json", {name: values.pop(name) for name in KEYS})
        os.environ.update(values)
        os.environ["SCOPE_RECOVERY_KEYS_FILE"] = str(root / "keys.json")
        os.environ["SCOPE_RECOVERY_RECIPIENT"] = required("SCOPE_RECOVERY_AGE_RECIPIENT")
        return capture.main(snapshot_provider=lambda destination, include_cache: railway.snapshot(maintenance, destination, include_cache))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception:
        print("Recovery runtime collection failed; no complete recovery set was published.", file=sys.stderr)
        raise SystemExit(1)
