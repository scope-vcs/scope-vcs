#!/usr/bin/env python3
"""Extract compiled release data without trusting archive paths or file types."""

import os
import json
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile

BACKEND_FILES = {
    'LICENSE', 'NOTICE', 'third-party-rust.txt', 'scope-maintenance',
    'scope-smoke-seed',
}
with (Path(__file__).resolve().parents[1] / 'deployment-services.json').open() as manifest_file:
    services = json.load(manifest_file)['services']
BACKEND_FILES.update(
    definition['artifact']['binary']
    for service in services.values()
    if (definition := service['deployment'])['backend'] and definition['artifact']['kind'] == 'binary'
)


def archive_path(member, kind):
    name = member.name
    if not name or '\x00' in name or '\\' in name or name.startswith('/'):
        raise ValueError(f'Unsafe archive path: {name!r}')
    if '..' in name.split('/'):
        raise ValueError(f'Archive path traverses outside its root: {name!r}')
    path = PurePosixPath(name)
    if not path.parts:
        if member.isdir():
            return None
        raise ValueError('Archive root must be a directory')
    if kind == 'backend':
        if len(path.parts) != 1 or path.name not in BACKEND_FILES or not member.isfile():
            raise ValueError(f'Unexpected backend archive entry: {name!r}')
    elif path.parts[0] != '.output' or (len(path.parts) == 1 and not member.isdir()):
        raise ValueError(f'Web archive entry is outside .output: {name!r}')
    return path


def safe_destination(destination):
    destination = Path(os.path.abspath(destination))
    for ancestor in [destination, *destination.parents]:
        if ancestor.is_symlink():
            raise ValueError(f'Extraction destination has a symlink: {ancestor}')
    if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
        raise ValueError('Extraction destination must be absent or an empty directory')
    return destination


def extract_release(kind, archive, destination):
    if kind not in ('backend', 'web'):
        raise ValueError('Release kind must be backend or web')
    destination = safe_destination(destination)
    with tarfile.open(archive, mode='r:gz') as release:
        entries = []
        paths = set()
        regular_files = set()
        # Validate all entries first. A malicious late entry cannot leave an
        # earlier credential link or executable behind for the publishing step.
        for member in release:
            if not (member.isfile() or member.isdir()) or member.issparse():
                raise ValueError(f'Archive links and special files are forbidden: {member.name!r}')
            path = archive_path(member, kind)
            if path is None:
                continue
            if path in paths:
                raise ValueError(f'Duplicate archive path: {member.name!r}')
            paths.add(path)
            if member.isfile():
                regular_files.add(path)
            entries.append((member, path))
        if not regular_files:
            raise ValueError('Release archive has no runtime files')
        if kind == 'web' and PurePosixPath('.output/server/index.mjs') not in regular_files:
            raise ValueError('Web release is missing .output/server/index.mjs')
        for _, path in entries:
            if any(parent in regular_files for parent in path.parents):
                raise ValueError(f'Archive file is also a parent directory: {str(path)!r}')
        destination.mkdir(parents=True, exist_ok=True)
        for member, path in entries:
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            mode = 0o755 if member.mode & 0o111 else 0o644
            with release.extractfile(member) as source:
                descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
                with os.fdopen(descriptor, 'wb') as output:
                    shutil.copyfileobj(source, output)


def main():
    if len(sys.argv) != 4:
        raise ValueError('usage: extract-railway-release.py backend|web ARCHIVE DESTINATION')
    extract_release(*sys.argv[1:])


if __name__ == '__main__':
    try:
        main()
    except (OSError, tarfile.TarError, ValueError) as error:
        print(f'Release extraction failed: {error}', file=sys.stderr)
        sys.exit(1)
