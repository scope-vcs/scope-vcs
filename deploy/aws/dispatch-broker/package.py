"""Build a deterministic Lambda ZIP using the AWS-managed Python SDK."""

import hashlib
import sys
import zipfile
from pathlib import Path


def package(destination):
    source = Path(__file__).resolve().parent
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    modules = ("handler.py", "journal.py", "lifecycle.py", "protocol.py", "provider.py", "settings.py")
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name in modules:
            content = (source / name).read_bytes()
            compile(content, name, "exec")
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, content)
    return hashlib.sha256(destination.read_bytes()).hexdigest()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 package.py OUTPUT.zip")
    print(package(sys.argv[1]))
