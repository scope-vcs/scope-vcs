import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from triage import session
from triage.policy import Observation, timestamp
from triage.store import Store


START = '2026-10-08T12:00:00+00:00'
END = '2026-10-08T12:10:00+00:00'
NOW = '2026-10-08T12:20:00.000000+00:00'
CATALOG = {'inheritedProviderInstanceId': 'provider', 'inheritedModel': 'model', 'providers': [{'providerInstanceId': 'provider', 'canRunChildTask': True, 'canRunCrossProviderChildTask': True, 'models': [{'id': 'model', 'options': []}, {'id': 'other-model', 'options': []}]}]}


def observation(index=1):
    return Observation('github', 'event-' + str(index), END, 'ci', 'backend', 'check', 'failure-' + str(index), 'https://github.com/scope-vcs/scope-vcs/actions/runs/' + str(index))


def diagnosis(claim):
    return {'facts': [{'observation': claim['packet']['observations'][0]['event_id'], 'finding': 'observed_failure'}], 'hypothesis': 'unknown', 'confidence': 'low', 'reproduction': 'unknown', 'impact': 'unknown', 'severity': 'unknown', 'owner': 'unknown', 'next_action': 'collect_sanitized_evidence', 'duplicate_candidates': []}


def receipt(claim, **changes):
    return {'taskId': 'task-' + claim['request_id'], 'status': 'completed', 'hasPendingChildRuns': False, 'workState': 'result_available', 'summary': json.dumps(diagnosis(claim)), **changes}


class SessionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / 'ledger.sqlite'

    def seed(self, store, count=1):
        store.ingest('github-runs-created:scope-vcs/scope-vcs', START, END, [observation(index) for index in range(1, count + 1)], NOW)

    def prepare(self, store, now=NOW, **target):
        with patch.object(session, 'utc_now', return_value=timestamp(now)):
            return session.prepare(store, CATALOG, **target)

    def test_sweep_malformed_batch_preserves_progress_and_reports_only_closed_reason(self):
        private = 'private-token-from-upstream'
        with Store(self.path) as store:
            self.seed(store)
            with patch.object(session, 'utc_now', return_value=NOW), patch.object(session.sources, 'github_runs', return_value=[observation(2), {'private': private}]):
                outcome = session.sweep(store, ['github-runs-created'], START, NOW, 'environment')
            self.assertEqual(outcome[0]['status'], 'degraded')
            self.assertEqual(outcome[0]['reason'], 'invalid_response')
            status = store.status()
            self.assertEqual(status['sources'][0]['cursor'], timestamp(END))
            self.assertEqual(status['sources'][0]['status'], 'invalid_response')
            self.assertEqual(len(status['incidents']), 1)
            self.assertNotIn(private, json.dumps({'outcome': outcome, 'status': status}))
        with Store(self.path) as reopened:
            self.assertEqual(reopened.cursor('github-runs-created:scope-vcs/scope-vcs'), timestamp(END))

    def test_prepare_emits_each_dispatch_once_and_retains_ambiguous_ownership(self):
        with Store(self.path) as store:
            self.seed(store, 3)
            first = self.prepare(store)
            self.assertEqual(len(first), 2)
            self.assertEqual({item['tool'] for item in first}, {'delegate_task'})
            self.assertEqual(len({item['arguments']['clientRequestId'] for item in first}), 2)
        with Store(self.path) as store:
            replay = self.prepare(store, model='other-model')
            self.assertEqual({item.get('action') for item in replay}, {'reconcile_unbound_dispatch'})
            self.assertEqual({item['request_id'] for item in replay}, {item['request_id'] for item in first})
            self.assertEqual({item['target']['model'] for item in store.active()}, {'model'})
            self.assertEqual(len(store.active()), 2)
            self.assertEqual(sum(item['state'] == 'pending' for item in store.status()['incidents']), 1)
            for claim in store.active():
                session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim, status='running', workState='working'), NOW)
            at_deadline = self.prepare(store, now=store.active()[0]['deadline'])
            self.assertEqual({item['tool'] for item in at_deadline}, {'task_cancel'})
            for request in at_deadline:
                self.assertEqual(set(request['arguments']), {'taskId'})
                self.assertIn(request['arguments']['taskId'], {claim['task_id'] for claim in store.active()})
            self.assertEqual(len(store.active()), 2)

    def test_unavailable_catalog_preserves_existing_task_controls(self):
        with Store(self.path) as store:
            self.seed(store, 3)
            self.prepare(store)
            for claim in store.active():
                session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim, status='running', workState='working'), NOW)
            for now, tool, catalog in [(now, tool, catalog) for now, tool in [(NOW, 'task_status'), (store.active()[0]['deadline'], 'task_cancel')] for catalog in ({}, {'providers': [{}]}, {'providers': None})]:
                with patch.object(session, 'utc_now', return_value=now):
                    requests = session.prepare(store, catalog)
                controls = [item for item in requests if 'tool' in item]
                self.assertEqual(len(controls), 2)
                self.assertEqual({item['tool'] for item in controls}, {tool})
                self.assertEqual({item['arguments']['taskId'] for item in controls}, {item['task_id'] for item in store.active()})
                self.assertTrue(any(item.get('action') == 'new_work_unavailable' for item in requests))
                self.assertEqual(sum(item['state'] == 'pending' for item in store.status()['incidents']), 1)

    def test_confirm_absent_releases_only_matching_unbound_claim_without_replay(self):
        with Store(self.path) as store:
            self.seed(store, 3)
            self.prepare(store)
            absent, bound = store.active()
            store.bind(bound['fingerprint'], bound['generation'], 'existing-task')
        automation = Path(__file__).resolve().parent
        command = [sys.executable, '-B', '-m', 'triage.session', '--state', str(self.path), 'confirm-absent']
        def invoke(claim, request_id=None, generation=None):
            return subprocess.run(command + ['--fingerprint', claim['fingerprint'], '--generation', str(generation or claim['generation']), '--request-id', request_id or claim['request_id']], cwd=automation, capture_output=True, text=True)
        for claim, request_id, generation in [(absent, 'wrong-request', None), (absent, None, absent['generation'] + 1), (bound, None, None)]:
            result = invoke(claim, request_id, generation)
            self.assertEqual(result.returncode, 2)
            with Store(self.path) as store:
                self.assertEqual(len(store.active()), 2)
        result = invoke(absent)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['status'], 'blocked')
        with Store(self.path) as store:
            incidents = {item['fingerprint']: item for item in store.status()['incidents']}
            self.assertEqual(incidents[absent['fingerprint']]['state'], 'blocked')
            self.assertEqual(incidents[absent['fingerprint']]['occurrences'], 1)
            self.assertEqual(store.drafts(), [])
            requests = self.prepare(store)
            self.assertEqual(len(store.active()), 2)
            self.assertNotIn(absent['fingerprint'], {item['fingerprint'] for item in requests})
            self.assertEqual({item['tool'] for item in requests}, {'delegate_task', 'task_status'})
            with self.assertRaises(ValueError):
                session.apply_receipt(store, absent['fingerprint'], absent['generation'], receipt(absent), NOW)
        self.assertEqual(invoke(absent).returncode, 2)

    def test_receive_requires_final_result_and_confirmed_no_live_children(self):
        with Store(self.path) as store:
            self.seed(store)
            self.prepare(store)
            claim = store.active()[0]
            for changes in ({'status': 'running', 'workState': 'working'}, {'hasPendingChildRuns': True}, {'hasPendingChildRuns': None}, {'workState': 'working'}, {'status': 'cancel_requested'}):
                with self.subTest(changes=changes):
                    result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim, **changes), NOW)
                    self.assertEqual(result['status'], 'active')
                    self.assertEqual(len(store.active()), 1)
                    self.assertEqual(store.drafts(), [])
            result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim), NOW)
            self.assertEqual(result['status'], 'draft')
            self.assertEqual(store.active(), [])
        with Store(self.path) as store:
            self.assertEqual(store.drafts()[0]['result'], diagnosis(claim))

    def test_expired_result_cannot_create_draft_and_cancel_ack_does_not_release_slot(self):
        with Store(self.path) as store:
            self.seed(store)
            self.prepare(store)
            claim = store.active()[0]
            result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim, status='cancel_requested', hasPendingChildRuns=None), claim['deadline'])
            self.assertEqual(result['status'], 'active')
            self.assertEqual(len(store.active()), 1)
            result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim), claim['deadline'])
            self.assertEqual(result['status'], 'blocked')
            self.assertEqual(store.active(), [])
            self.assertEqual(store.drafts(), [])
            self.assertEqual(store.status()['incidents'][0]['state'], 'blocked')

    def test_confirmed_terminal_stops_release_capacity_only_without_pending_children(self):
        for state in ('failed', 'cancelled', 'interrupted'):
            with self.subTest(state=state), Store(self.path.with_name(state + '.sqlite')) as store:
                self.seed(store)
                self.prepare(store)
                claim = store.active()[0]
                stopped = receipt(claim, status=state, summary=None, hasPendingChildRuns=True)
                result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], stopped, NOW)
                self.assertEqual(result['status'], 'active')
                self.assertEqual(len(store.active()), 1)
                stopped['hasPendingChildRuns'] = False
                result = session.apply_receipt(store, claim['fingerprint'], claim['generation'], stopped, NOW)
                self.assertEqual(result['status'], 'blocked')
                self.assertEqual(store.active(), [])
                self.assertEqual(store.drafts(), [])

    def test_invalid_completed_summary_is_rejected_without_losing_dispatch_ownership(self):
        with Store(self.path) as store:
            self.seed(store)
            self.prepare(store)
            claim = store.active()[0]
            with self.assertRaises(ValueError):
                session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim, summary='private-token-invalid-output'), NOW)
            self.assertEqual(len(store.active()), 1)
            self.assertEqual(store.drafts(), [])
            self.assertNotIn('private-token-invalid-output', json.dumps(store.status()))

    def test_cli_reopens_persisted_draft_and_returns_generic_errors(self):
        with Store(self.path) as store:
            self.seed(store)
            self.prepare(store)
            claim = store.active()[0]
            session.apply_receipt(store, claim['fingerprint'], claim['generation'], receipt(claim), NOW)
        automation = Path(__file__).resolve().parent
        result = subprocess.run([sys.executable, '-m', 'triage.session', '--state', str(self.path), 'drafts'], cwd=automation, capture_output=True, text=True, check=True)
        drafts = json.loads(result.stdout)
        self.assertEqual(drafts[0]['incident'], claim['fingerprint'])
        self.assertIn('scope-incident:' + claim['fingerprint'], drafts[0]['markdown'])
        self.assertIn('Reproduction: unknown', drafts[0]['markdown'])
        self.assertEqual(drafts[0]['occurrences'], 1)
        invalid = self.path.parent / 'invalid.json'
        for value in ('private-token-bad-json', '[]', 'null', '42', '{"structuredContent": []}'):
            with self.subTest(value=value):
                invalid.write_text(value)
                result = subprocess.run([sys.executable, '-m', 'triage.session', '--state', str(self.path), 'receive', '--fingerprint', claim['fingerprint'], '--generation', str(claim['generation']), '--receipt', str(invalid)], cwd=automation, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn('state retained', result.stderr)
                self.assertNotIn('private-token-bad-json', result.stdout + result.stderr)
                self.assertNotIn('Traceback', result.stderr)


if __name__ == '__main__':
    unittest.main()
