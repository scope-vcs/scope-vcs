"""Durable launch, reconciliation, and terminal cleanup for a single attempt."""

import hmac
import time

from protocol import Denied, Pending, request, token_hash
from provider import ProviderRejected


CONSISTENCY_SECONDS = 300


class Broker:
    def __init__(self, journal, authority, provider, clock=time.time):
        self.journal = journal
        self.authority = authority
        self.provider = provider
        self.clock = clock

    def handle(self, event, owner):
        record = None
        try:
            command = request(event)
            record = self.journal.acquire(command["attempt_id"], owner, int(self.clock()))
            if command["action"] == "start":
                return self.start(command, record, owner)
            return self.stop(command, record, owner)
        except Denied:
            # Rejection promises that no task can exist. A failed authorization
            # on an existing attempt cannot make that promise.
            if record and record.get("phase") not in (None, "stopped"):
                return {"status": "ambiguous", "message": "attempt requires reconciliation"}
            return {"status": "rejected", "message": "attempt dispatch was rejected"}
        except Exception:
            # Never return AWS errors or caller payloads; they can contain
            # bootstrap values, registry references, or authorization headers.
            return {"status": "ambiguous", "message": "attempt requires reconciliation"}
        finally:
            if record is not None:
                try:
                    self.journal.release(record["attempt_id"], owner)
                except Exception:
                    # The lock expires only after the invocation's hard timeout.
                    pass

    def start(self, command, record, owner):
        phase = record.get("phase")
        if phase in ("stopping", "stopped"):
            raise Denied("attempt dispatch is closed")
        hashed = token_hash(command["bootstrap_token"])
        if phase and not hmac.compare_digest(record["bootstrap_hash"], hashed):
            raise Denied("attempt credential changed")
        if phase == "running":
            # This returns the prior result without issuing a new launch or
            # consuming the bootstrap token that the runtime already exchanged.
            return {"status": "started", "task_arn": record["task_arn"]}
        if phase == "launching":
            tasks = self.provider.find(record["attempt_id"])
            if len(tasks) == 1:
                record.update(phase="running", task_arn=tasks[0])
                self.journal.save(record, owner)
                return {"status": "started", "task_arn": tasks[0]}
            # Never repeat RunTask after an uncertain response. ECS tokens
            # expire, and a missing task can still be an eventual-consistency gap.
            raise Pending("launch outcome is unknown")
        authorized = self.authority.authorize(command)
        if authorized["deadline_unix"] <= int(self.clock()):
            raise Denied("attempt deadline elapsed")
        if phase:
            if record["image"] != authorized["image"] or record["deadline_unix"] != authorized["deadline_unix"]:
                raise Denied("attempt specification changed")
        else:
            record.update(
                phase="preparing", image=authorized["image"],
                deadline_unix=authorized["deadline_unix"], bootstrap_hash=hashed,
                setup_at=int(self.clock()),
            )
            self.journal.save(record, owner)
        if record["phase"] == "preparing":
            specification = self.provider.prepare(record, command["bootstrap_token"])
            record.update(phase="registered", launch_specification=specification)
            self.journal.save(record, owner)
        # Check cancellation again after potentially slow image/secret setup.
        self.authority.authorize(command)
        record.update(phase="launching", launch_at=int(self.clock()))
        self.journal.save(record, owner)
        try:
            task = self.provider.launch(record["launch_specification"])
        except ProviderRejected:
            # Durable setup still owns cleanup. Keep reconciliation responsible
            # until definitions and the secret are positively cleaned up.
            record.update(phase="stopping", launch_rejected=True)
            self.journal.save(record, owner)
            self.provider.cleanup(record)
            record.update(phase="stopped")
            self.journal.save(record, owner)
            return {"status": "rejected", "message": "ECS rejected this attempt"}
        record.update(phase="running", task_arn=task)
        self.journal.save(record, owner)
        return {"status": "started", "task_arn": task}

    def stop(self, command, record, owner):
        self.authority.authorize(command)
        if record.get("phase") == "stopped":
            return {"status": "stopped"}
        record.update(phase="stopping")
        self.journal.save(record, owner)
        tasks = set(self.provider.find(record["attempt_id"]))
        if record.get("task_arn"):
            tasks.add(record["task_arn"])
        # Persist discoveries before stopping, so stopped tasks disappearing
        # from ListTasks cannot make a lost stop response lose task ownership.
        tasks.update(record.get("reconciling_tasks", []))
        record["reconciling_tasks"] = sorted(tasks)
        self.journal.save(record, owner)
        all_stopped = True
        missing = False
        for task in sorted(tasks):
            status = self.provider.stop(task)
            missing = missing or status == "missing"
            all_stopped = status in ("stopped", "missing") and all_stopped
        if not all_stopped:
            raise Pending("ECS has not confirmed every task stopped")
        # A crashed launch or registration can become visible after the caller
        # loses its response. Give discovery the same five-minute window as the
        # previous provider implementation before declaring absence.
        uncertain_at = record.get("launch_at", record.get("setup_at", 0))
        if missing and int(self.clock()) < uncertain_at + CONSISTENCY_SECONDS:
            raise Pending("waiting for known task visibility")
        if uncertain_at and not record.get("task_arn") and not record.get("launch_rejected"):
            if int(self.clock()) < uncertain_at + CONSISTENCY_SECONDS:
                raise Pending("waiting for ECS consistency")
        self.provider.cleanup(record)
        record.update(phase="stopped")
        self.journal.save(record, owner)
        return {"status": "stopped"}
