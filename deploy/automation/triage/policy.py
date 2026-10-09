from __future__ import annotations

import hashlib
import json
import re
from dataclasses import asdict, dataclass
from datetime import datetime, timezone

MAX_INVESTIGATORS = 2
INVESTIGATION_SECONDS = 900
MAX_EVIDENCE = 20
REPOSITORY = 'scope-vcs/scope-vcs'


def timestamp(value: str) -> str:
    parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if parsed.tzinfo is None:
        raise ValueError('Timestamp must include a timezone')
    return parsed.astimezone(timezone.utc).isoformat(timespec='microseconds')


def digest(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def identifier(value: str, pattern: str, name: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(pattern, value):
        raise ValueError(f'Invalid {name}')
    return value


@dataclass(frozen=True)
class Observation:
    source: str
    event_id: str
    occurred_at: str
    environment: str
    service: str
    operation: str
    signature: str
    reference: str
    release: str = ''
    deployment: str = ''
    trace_id: str = ''
    release_owned: bool = False
    indexed_at: str = ''

    def __post_init__(self):
        identifier(self.source, r'railway|github|browser|release', 'source')
        identifier(self.environment, r'production|ci|staging', 'environment')
        for name in ('event_id', 'service', 'operation', 'signature'):
            identifier(getattr(self, name), r'[A-Za-z0-9_.:/{} -]{1,200}', name)
        identifier(self.release, r'(?:[0-9a-f]{40}|[0-9a-f]{64})?', 'release')
        identifier(self.deployment, r'[a-zA-Z0-9-]{0,64}', 'deployment')
        identifier(self.trace_id, r'(?:[0-9a-f]{32})?', 'trace ID')
        if not re.fullmatch(r'(?:https://github.com/scope-vcs/scope-vcs/(?:actions/runs/[0-9]+(?:/attempts/[0-9]+)?|issues/[0-9]+)|railway:trace:[0-9a-f]{32}|posthog:event:[a-zA-Z0-9-]+|release:run:[0-9]+)', self.reference):
            raise ValueError('Invalid evidence reference')
        object.__setattr__(self, 'occurred_at', timestamp(self.occurred_at))
        if self.indexed_at:
            object.__setattr__(self, 'indexed_at', timestamp(self.indexed_at))

    @property
    def key(self) -> str:
        return digest([self.source, self.environment, self.event_id])

    @property
    def fingerprint(self) -> str:
        return 'v1:' + digest([self.environment, self.service, self.operation, self.signature])

    def record(self) -> dict:
        return asdict(self)


def evidence_packet(incident: dict, observations: list[dict]) -> dict:
    return {
        'incident': incident['fingerprint'],
        'provisional': any(item['source'] == 'browser' for item in observations),
        'observations': [Observation(**item).record() for item in observations[:MAX_EVIDENCE]],
    }
