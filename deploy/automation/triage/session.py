from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

from . import sources
from .investigate import cancellation_request, delegation_request, render_draft, select_target, status_request, validate_result
from .policy import timestamp
from .store import Store

SOURCES = ('railway', 'browser', 'github-issues', 'github-runs-created')
TERMINAL = {'completed', 'failed', 'cancelled', 'interrupted'}


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='microseconds')


def sweep(store, selected, since, until, environment_id):
    now = utc_now()
    end = timestamp(until or now)
    initial = timestamp(since or (datetime.fromisoformat(end) - timedelta(hours=1)).isoformat())
    if initial > end or end > now:
        raise ValueError('Invalid collection interval')
    results = []
    for source in selected:
        key = source + ':' + (environment_id if source == 'railway' else 'production' if source == 'browser' else 'scope-vcs/scope-vcs')
        cursor = store.cursor(key)
        start = timestamp((datetime.fromisoformat(cursor) - timedelta(minutes=5)).isoformat()) if cursor else initial
        if start > end:
            raise ValueError('Collection end precedes durable progress')
        budget = sources.Budget()
        try:
            if source == 'railway':
                rows = sources.railway(start, end, environment_id, budget)
            else:
                collector = {'browser': sources.browser, 'github-issues': sources.github_issues, 'github-runs-created': sources.github_runs}[source]
                rows = collector(start, end, budget)
            store.ingest(key, start, end, rows, now)
            results.append({'source': key, 'status': 'complete', 'observations': len(rows), 'through': end})
        except (sources.SourceError, ValueError, KeyError, TypeError, AttributeError) as error:
            reason = error.reason if isinstance(error, sources.SourceError) else 'invalid_response'
            store.degraded(key, reason, now)
            results.append({'source': key, 'status': 'degraded', 'reason': reason})
    return results


def prepare(store, capabilities, provider=None, model=None, options=None):
    target = select_target(capabilities, provider, model, options)
    now = utc_now()
    while store.claim(now, target) is not None:
        pass
    requests = []
    for claim in store.active():
        identity = {name: claim[name] for name in ('fingerprint', 'generation', 'request_id', 'deadline')}
        expired = claim['deadline'] <= now or claim['release_owned']
        if claim['task_id']:
            tool = 'task_cancel' if expired else 'task_status'
            arguments = cancellation_request(claim) if expired else status_request(claim)
        elif expired:
            requests.append({**identity, 'action': 'reconcile_unbound_dispatch', 'blocked': True})
            continue
        else:
            tool = 'delegate_task'
            saved = claim['target']
            arguments = delegation_request(claim, saved['providerInstanceId'], saved['model'], saved['options'])
        requests.append({**identity, 'tool': tool, 'arguments': arguments})
    return requests


def receipt_data(path):
    data = json.loads(Path(path).read_text())
    if data.get('isError'):
        raise ValueError('T3 request did not succeed')
    return data.get('structuredContent', data)


def apply_receipt(store, fingerprint, generation, receipt, now):
    claims = [item for item in store.active() if item['fingerprint'] == fingerprint and item['generation'] == generation]
    if len(claims) != 1:
        raise ValueError('Stale investigation ownership')
    claim = claims[0]
    task_id = receipt['taskId']
    store.bind(fingerprint, generation, task_id, now)
    if receipt.get('status') not in TERMINAL or receipt.get('hasPendingChildRuns') is not False or receipt.get('workState') != 'result_available':
        return {'status': 'active', 'task_id': task_id}
    if receipt['status'] == 'completed' and receipt.get('workState') == 'result_available' and now < claim['deadline'] and not claim['release_owned']:
        result = validate_result(receipt['summary'], claim['packet'])
        store.finish(fingerprint, generation, task_id, result, now)
        return {'status': 'draft', 'task_id': task_id}
    store.stopped(fingerprint, generation, task_id, now)
    return {'status': 'blocked', 'task_id': task_id}


def main(argv=None):
    parser = argparse.ArgumentParser(description='Read-only incident intake and local draft investigations')
    parser.add_argument('--state', type=Path, default=Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'scope-triage/ledger.sqlite3')
    commands = parser.add_subparsers(dest='command', required=True)
    collect = commands.add_parser('sweep')
    collect.add_argument('--source', choices=SOURCES + ('all',), default='all')
    collect.add_argument('--since')
    collect.add_argument('--until')
    collect.add_argument('--environment-id', default=json.loads((sources.ROOT / '.github/deployment-services.json').read_text())['environments']['production']['environmentId'])
    commands.add_parser('status')
    prepare_parser = commands.add_parser('prepare')
    prepare_parser.add_argument('--capabilities', required=True, type=Path)
    prepare_parser.add_argument('--provider')
    prepare_parser.add_argument('--model')
    prepare_parser.add_argument('--options', default='{}')
    receive = commands.add_parser('receive')
    receive.add_argument('--fingerprint', required=True)
    receive.add_argument('--generation', required=True, type=int)
    receive.add_argument('--receipt', required=True, type=Path)
    commands.add_parser('drafts')
    args = parser.parse_args(argv)
    with Store(args.state) as store:
        if args.command == 'sweep':
            result = sweep(store, SOURCES if args.source == 'all' else [args.source], args.since, args.until, args.environment_id)
        elif args.command == 'prepare':
            result = prepare(store, receipt_data(args.capabilities), args.provider, args.model, json.loads(args.options))
        elif args.command == 'receive':
            result = apply_receipt(store, args.fingerprint, args.generation, receipt_data(args.receipt), utc_now())
        elif args.command == 'drafts':
            result = [{'incident': item['fingerprint'], 'occurrences': item['occurrences'], 'release_owned': bool(item['release_owned']),
                       'markdown': render_draft(item['packet'], item['result'])} for item in store.drafts()]
        else:
            result = store.status(utc_now())
        print(json.dumps(result, indent=2))
    return 1 if args.command == 'sweep' and any(item['status'] == 'degraded' for item in result) else 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, KeyError, TypeError, OSError):
        print('Triage operation failed; state retained. Check configuration and input schema.', file=sys.stderr)
        sys.exit(2)
