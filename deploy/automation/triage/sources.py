from __future__ import annotations

import json
import os
import re
import subprocess
import time
import urllib.request
from datetime import datetime, timedelta
from pathlib import Path
from urllib.parse import urlencode

from .policy import Observation, REPOSITORY, digest, timestamp

LIMIT = 500
MAX_REQUESTS = 40
SWEEP_SECONDS = 120
ROOT = Path(__file__).resolve().parents[3]


class SourceError(RuntimeError):
    def __init__(self, reason: str):
        self.reason = reason
        super().__init__(reason)


class Budget:
    def __init__(self):
        self.remaining = MAX_REQUESTS
        self.deadline = time.monotonic() + SWEEP_SECONDS

    def timeout(self):
        self.remaining -= 1
        remaining = self.deadline - time.monotonic()
        if self.remaining < 0 or remaining <= 0:
            raise SourceError('incomplete')
        return min(30, remaining)

    def command(self, args: list[str], ndjson=False):
        try:
            result = subprocess.run(args, capture_output=True, text=True, timeout=self.timeout())
            if result.returncode:
                raise SourceError('unavailable')
            return [json.loads(line) for line in result.stdout.splitlines() if line.strip()] if ndjson else json.loads(result.stdout)
        except (subprocess.TimeoutExpired, OSError):
            raise SourceError('unavailable') from None
        except (ValueError, TypeError):
            raise SourceError('invalid_response') from None


def bounded_windows(fetch, start: str, end: str) -> list[dict]:
    rows = fetch(start, end)
    if len(rows) < LIMIT:
        return rows
    left, right = datetime.fromisoformat(start), datetime.fromisoformat(end)
    if right - left <= timedelta(seconds=1):
        raise SourceError('incomplete')
    middle = (left + (right - left) / 2).isoformat(timespec='microseconds')
    return bounded_windows(fetch, start, middle) + bounded_windows(fetch, middle, end)


def sha(value) -> str:
    return value if isinstance(value, str) and re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', value) else ''


def opaque(value) -> str:
    return digest(value)[:24]


def railway(start, end, environment_id, budget):
    start, end = timestamp(start), timestamp(end)
    manifest = json.loads((ROOT / '.github/deployment-services.json').read_text())
    environments = {item['environmentId']: name for name, item in manifest['environments'].items()}
    environment = environments.get(environment_id)
    if environment not in {'production', 'staging'}:
        raise SourceError('configuration')
    services = {item['id']: name for name, item in manifest['services'].items()}
    scope = ['--project', manifest['railway']['projectId'], '--environment', environment_id]
    status = budget.command(['railway', 'trace', 'status', '--all', *scope, '--json'])
    expected = set(services)
    enabled = {item['id'] for item in status['services'] if item['tracingEnabled'] and item.get('lastServiceSpanAt')}
    if not expected <= enabled:
        raise SourceError('configuration')
    rows = bounded_windows(lambda a, b: budget.command([
        'railway', 'trace', 'list', '--all', *scope, '--since', a, '--until', b,
        '--errors', '--limit', str(LIMIT), '--json'], ndjson=True), start, end)
    observations = []
    for trace_id in sorted({item['traceId'] for item in rows}):
        if not re.fullmatch(r'[0-9a-f]{32}', trace_id):
            raise SourceError('invalid_response')
        spans = budget.command(['railway', 'trace', 'get', trace_id, *scope, '--max-spans', '2000', '--json'], ndjson=True)
        if len(spans) >= 2000:
            raise SourceError('incomplete')
        for span in spans:
            if span.get('traceId') != trace_id or not re.fullmatch(r'[0-9a-f]{16}', span.get('spanId', '')):
                raise SourceError('invalid_response')
            if span['statusCode'] != 'ERROR' and span['statusCode'] != 2:
                continue
            occurred = timestamp(span['startedAt'])
            if not start <= occurred <= end:
                continue
            service = services.get(span.get('serviceId'))
            if service is None:
                raise SourceError('invalid_response')
            attrs = span.get('spanAttributes') or {}
            if attrs.get('http.route') in {'/healthz', '/readyz'}:
                continue
            operation = 'operation:' + opaque(span['name'])
            observations.append(Observation(
                'railway', trace_id + ':' + span['spanId'], occurred, environment, service,
                operation, 'span_error', 'railway:trace:' + trace_id,
                sha((span.get('resourceAttributes') or {}).get('service.version')),
                span.get('deploymentId') or '', trace_id))
    return observations


def github(path, budget):
    return budget.command(['gh', 'api', f'repos/{REPOSITORY}/{path}'])


def github_issues(start, end, budget):
    start, end = timestamp(start), timestamp(end)
    observations = []
    page = 1
    while True:
        rows = github('issues?' + urlencode({'state': 'open', 'since': start, 'sort': 'updated', 'direction': 'asc', 'per_page': 100, 'page': page}), budget)
        for item in rows:
            if 'pull_request' in item or item['state'] != 'open' or not start <= timestamp(item['updated_at']) <= end:
                continue
            labels = {label['name'] for label in item['labels']}
            if not labels & {'bug', 'release-flake'}:
                continue
            number = int(item['number'])
            observations.append(Observation(
                'github', f'issue:{number}:' + opaque(item['updated_at']), item['updated_at'], 'ci', 'github',
                f'issue:{number}', 'reported_bug', f'https://github.com/{REPOSITORY}/issues/{number}',
                release_owned='release-flake' in labels))
        if len(rows) < 100 or (rows and timestamp(rows[-1]['updated_at']) > end):
            return observations
        page += 1


def github_runs(start, end, budget):
    start, end = timestamp(start), timestamp(end)
    observations = []
    page = 1
    while True:
        response = github('actions/runs?' + urlencode({'created': start + '..' + end, 'per_page': 100, 'page': page}), budget)
        if response['total_count'] >= 1000:
            raise SourceError('incomplete')
        rows = response['workflow_runs']
        for run in rows:
            if run['status'] != 'completed':
                raise SourceError('incomplete')
            for attempt in range(1, int(run['run_attempt']) + 1):
                item = github(f"actions/runs/{int(run['id'])}/attempts/{attempt}", budget)
                if item['status'] != 'completed':
                    raise SourceError('incomplete')
                if item['conclusion'] not in {'failure', 'timed_out', 'action_required', 'startup_failure'}:
                    continue
                run_id = int(item['id'])
                workflow_id = int(item['workflow_id'])
                observations.append(Observation(
                    'github', f'run:{run_id}:{attempt}', item['updated_at'], 'ci', 'github',
                    f'workflow:{workflow_id}', item['conclusion'],
                    f'https://github.com/{REPOSITORY}/actions/runs/{run_id}/attempts/{attempt}',
                    sha(item.get('head_sha')),
                    release_owned=item.get('path', '').split('@', 1)[0] in {'.github/workflows/release.yml', '.github/workflows/image-pin-refresh.yml'},
                    indexed_at=run['created_at']))
        if len(rows) < 100:
            return observations
        page += 1


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def browser(start, end, budget):
    start, end = timestamp(start), timestamp(end)
    project = os.environ.get('POSTHOG_PROJECT_ID', '')
    token = os.environ.get('POSTHOG_PERSONAL_API_KEY', '')
    host = os.environ.get('POSTHOG_APP_HOST', 'https://us.posthog.com')
    if not re.fullmatch(r'[0-9]+', project) or not token or host not in {'https://us.posthog.com', 'https://eu.posthog.com'}:
        raise SourceError('configuration')

    def fetch(a, b):
        query = ("SELECT uuid,timestamp,properties.error_kind,properties.error_origin,properties.route_name,properties.release "
                 "FROM events WHERE event='frontend_error' AND properties.environment='production' "
                 f"AND timestamp >= toDateTime('{a}') AND timestamp <= toDateTime('{b}') ORDER BY timestamp,uuid LIMIT {LIMIT}")
        request = urllib.request.Request(f'{host}/api/projects/{project}/query/',
            data=json.dumps({'query': {'kind': 'HogQLQuery', 'query': query}, 'refresh': 'blocking'}).encode(),
            headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
        try:
            with urllib.request.build_opener(NoRedirect()).open(request, timeout=budget.timeout()) as response:
                data = json.load(response)
        except (OSError, ValueError):
            raise SourceError('unavailable') from None
        if data.get('error') or not isinstance(data.get('results'), list):
            raise SourceError('incomplete')
        return data['results']

    observations = []
    kinds = {'abort_error', 'aggregate_error', 'dom_error', 'eval_error', 'range_error', 'reference_error', 'syntax_error', 'type_error', 'unknown_error', 'uri_error'}
    for event_id, occurred, kind, origin, route, release in bounded_windows(fetch, start, end):
        if kind not in kinds or origin not in {'hydration', 'promise', 'route', 'window'}:
            raise SourceError('invalid_response')
        if not isinstance(event_id, str) or not re.fullmatch(r'[a-zA-Z0-9-]{1,64}', event_id):
            raise SourceError('invalid_response')
        observations.append(Observation('browser', event_id, occurred, 'production', 'web',
            'route:' + opaque(route), kind + ':' + origin, 'posthog:event:' + event_id, sha(release)))
    return observations
