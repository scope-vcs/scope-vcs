#!/usr/bin/env python3
"""Own Linux local-dev sessions by boot/start identity and signal through pidfds."""
import json
import os
from pathlib import Path
import select
import signal
import sys
import time


def process_identity(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        if fields[0] == "Z":
            return None
        return {"pid": pid, "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                "start": fields[19], "group": int(fields[2]), "session": int(fields[3])}
    except (OSError, ValueError, IndexError):
        return None


def owned_identity(path):
    try:
        record = json.loads(path.read_text())
        if not isinstance(record, dict) or type(record.get("pid")) is not int or record["pid"] <= 1:
            return None
        current = process_identity(record["pid"])
        if current == record and current["group"] == current["session"] == current["pid"]:
            return current
    except (OSError, ValueError):
        pass
    return None


def record_process(path, pid):
    for _ in range(100):
        identity = process_identity(pid)
        if identity and identity["group"] == identity["session"] == pid:
            path.write_text(json.dumps(identity) + "\n")
            path.chmod(0o600)
            return
        time.sleep(0.01)
    raise RuntimeError(f"process {pid} did not start an owned session")


def session_handles(owner):
    handles = []
    try:
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            identity = process_identity(int(entry.name))
            if not identity or identity["session"] != owner["session"]:
                continue
            try:
                fd = os.pidfd_open(identity["pid"])
            except ProcessLookupError:
                continue
            # Opening the descriptor must not accidentally select a reused PID.
            if process_identity(identity["pid"]) != identity:
                os.close(fd)
            else:
                handles.append(fd)
        return handles
    except BaseException:
        for fd in handles:
            os.close(fd)
        raise


def stop_process(path):
    owner = owned_identity(path)
    if not owner:
        if path.exists():
            print(f"ignoring stale process record: {path}", file=sys.stderr)
        path.unlink(missing_ok=True)
        return
    handles = session_handles(owner)
    try:
        if owned_identity(path) != owner:
            return
        poller = select.poll()
        for fd in handles:
            poller.register(fd, select.POLLIN)
            try:
                signal.pidfd_send_signal(fd, signal.SIGTERM)
            except ProcessLookupError:
                pass
        pending = set(handles)
        deadline = time.monotonic() + 3
        while pending and time.monotonic() < deadline:
            for fd, _ in poller.poll(100):
                pending.discard(fd)
                poller.unregister(fd)
        for fd in pending:
            try:
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
    finally:
        for fd in handles:
            os.close(fd)
        path.unlink(missing_ok=True)


def main():
    action, filename, *args = sys.argv[1:]
    path = Path(filename)
    if action == "record":
        record_process(path, int(args[0]))
    elif action == "stop":
        stop_process(path)
    elif action == "status":
        identity = owned_identity(path)
        print(f"running pid {identity['pid']}" if identity else "stale" if path.exists() else "stopped")
    else:
        raise ValueError(f"unknown process action: {action}")


if __name__ == "__main__":
    main()
