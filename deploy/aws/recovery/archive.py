import subprocess
import tarfile
from pathlib import Path, PurePosixPath

from common import Incomplete, MAX_BYTES


def encrypt(root, recipient, destination):
    process = subprocess.Popen(["age", "--recipient", recipient, "--output", str(destination)], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        with tarfile.open(fileobj=process.stdin, mode="w|") as archive:
            for path in sorted(Path(root).rglob("*")):
                if path.is_file():
                    archive.add(path, arcname=path.relative_to(root).as_posix(), recursive=False)
        process.stdin.close()
        process.stdin = None
        _, error = process.communicate(timeout=1800)
        if process.returncode:
            raise Incomplete("recovery archive encryption failed")
    except BaseException:
        process.kill()
        process.wait()
        if process.stdin:
            process.stdin.close()
        if process.stderr:
            process.stderr.close()
        raise
    Path(destination).chmod(0o600)


def decrypt(source, identity, destination, max_bytes=MAX_BYTES + 1024**3):
    if Path(source).stat().st_size > max_bytes + 64 * 1024**2:
        raise Incomplete("encrypted archive exceeds the restore byte cap")
    destination = Path(destination)
    if destination.exists():
        raise Incomplete("restore destination must not already exist")
    destination.mkdir(parents=True, mode=0o700)
    process = subprocess.Popen(["age", "--decrypt", "--identity", str(identity), str(source)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    total = 0
    seen = set()
    try:
        with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
            for member in archive:
                name = PurePosixPath(member.name)
                if not member.isfile() or name.is_absolute() or ".." in name.parts or member.name in seen:
                    raise Incomplete("archive contains an unsafe or duplicate path")
                if member.name not in ("database.dump", "keys.json", "manifest.json") and not (len(name.parts) == 3 and name.parts[0] == "objects" and name.parts[1] in ("objects", "media", "cache") and len(name.parts[2]) == 64 and all(ch in "0123456789abcdef" for ch in name.parts[2])):
                    raise Incomplete("archive contains an unexpected file")
                total += member.size
                if total > max_bytes:
                    raise Incomplete("archive exceeds the restore byte cap")
                seen.add(member.name)
                target = destination / member.name
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as incoming, target.open("xb") as outgoing:
                    while chunk := incoming.read(1024**2):
                        outgoing.write(chunk)
                target.chmod(0o600)
        # Read through age's authenticated end, including tar padding, before
        # trusting any extracted bytes or starting database recovery.
        while process.stdout.read(1024**2):
            pass
        process.stdout.close()
        process.stdout = None
        _, error = process.communicate(timeout=1800)
        if process.returncode:
            raise Incomplete("archive decryption or authentication failed")
        if not {"database.dump", "keys.json", "manifest.json"} <= seen:
            raise Incomplete("archive is missing required recovery components")
    except BaseException:
        process.kill()
        process.wait()
        if process.stdout:
            process.stdout.close()
        if process.stderr:
            process.stderr.close()
        raise
