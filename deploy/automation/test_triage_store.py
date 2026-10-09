from dataclasses import replace
from datetime import datetime, timedelta, timezone
from pathlib import Path
import stat
import tempfile
import unittest

from triage.policy import Observation, timestamp
from triage.store import Store

TARGET = {'providerInstanceId': 'test', 'model': 'test', 'options': {}}
NOW = datetime(2026, 10, 8, 12, tzinfo=timezone.utc)


def at(seconds=0):
    return (NOW + timedelta(seconds=seconds)).isoformat()


def observation(event='one', signature='failure', release_owned=False):
    return Observation('github', event, at(1), 'ci', 'backend', 'check', signature,
                       'https://github.com/scope-vcs/scope-vcs/actions/runs/123', release_owned=release_owned)


def diagnosis(event='one'):
    return {'facts': [{'observation': event, 'finding': 'observed_failure'}],
            'hypothesis': 'unknown', 'confidence': 'low', 'reproduction': 'unknown',
            'impact': 'unknown', 'severity': 'unknown', 'owner': 'unknown',
            'next_action': 'collect_sanitized_evidence', 'duplicate_candidates': []}


class StoreTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / 'ledger' / 'triage.sqlite'

    def ingest(self, store, items):
        store.ingest('github:checks', at(), at(10), items, at(10))

    def test_reopened_overlap_and_degradation_preserve_unique_evidence_and_cursor(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
        with Store(self.path) as store:
            store.ingest('github:checks', at(5), at(20), [], at(20))
            self.ingest(store, [observation(), observation('two')])
            store.degraded('github:checks', 'unavailable', at(21))
            self.assertEqual(store.cursor('github:checks'), timestamp(at(20)))
            overview = store.status()
            self.assertEqual(overview['incidents'][0]['occurrences'], 2)
            self.assertEqual(overview['sources'][0]['status'], 'unavailable')
            with self.assertRaises(ValueError):
                store.ingest('github:checks', at(21), at(30), [], at(30))
            with self.assertRaises(ValueError):
                store.degraded('github:checks', 'raw private failure', at(21))
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.path.parent.stat().st_mode), 0o700)

    def test_collection_cursor_uses_index_time_and_evidence_keeps_failure_time(self):
        with Store(self.path) as store:
            failure = replace(observation(), occurred_at=at(100), indexed_at=at(1))
            store.ingest('github-runs-created', at(), at(10), [failure], at(110))
            claim = store.claim(at(110), TARGET)
            self.assertEqual(store.cursor('github-runs-created'), timestamp(at(10)))
            self.assertEqual(claim['packet']['observations'][0]['occurred_at'], timestamp(at(100)))

    def test_bad_batch_rolls_back_observations_and_cursor_together(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
            with self.assertRaises(ValueError):
                store.ingest('github:checks', at(), at(20), [observation('two', 'another'), object()], at(20))
            self.assertEqual(store.cursor('github:checks'), timestamp(at(10)))
            incidents = store.status()['incidents']
            self.assertEqual(len(incidents), 1)
            self.assertEqual(incidents[0]['occurrences'], 1)
            with self.assertRaises(ValueError):
                self.ingest(store, [observation(signature='conflicting')])
            self.assertEqual(len(store.status()['incidents']), 1)

    def test_connections_share_capacity_and_crashed_dispatch_intents_still_block(self):
        with Store(self.path) as first, Store(self.path) as second:
            self.ingest(first, [observation(str(i), str(i)) for i in range(3)])
            selected = {**TARGET}
            initial = first.claim(at(10), selected)
            self.assertIsNotNone(second.claim(at(10), TARGET))
            self.assertIsNone(first.claim(at(10), TARGET))
            selected['model'] = 'changed'
            persisted = next(item for item in first.active() if item['fingerprint'] == initial['fingerprint'])
            self.assertEqual(persisted['target']['model'], 'test')
        with Store(self.path) as store:
            self.assertIsNone(store.claim(at(1000), TARGET))
            durable = next(item for item in store.active() if item['fingerprint'] == initial['fingerprint'])
            self.assertEqual(durable['request_id'], initial['request_id'])
            self.assertEqual(durable['target'], TARGET)
            self.assertIsNone(durable['task_id'])
            with self.assertRaises(ValueError):
                store.stopped(initial['fingerprint'], initial['generation'], None)
            self.assertEqual(len(store.active()), 2)
            store.bind(initial['fingerprint'], initial['generation'], 'task-one')
            bound = next(item for item in store.active() if item['fingerprint'] == initial['fingerprint'])
            self.assertEqual(bound['task_id'], 'task-one')

    def test_expiry_requires_confirmed_stop_and_fences_results(self):
        with Store(self.path) as store:
            self.ingest(store, [observation(str(i), str(i)) for i in range(3)])
            claim = store.claim(at(10), TARGET)
            store.claim(at(10), TARGET)
            fingerprint, generation = claim['fingerprint'], claim['generation']
            result = diagnosis(claim['packet']['observations'][0]['event_id'])
            store.bind(fingerprint, generation, 'task-one')
            for candidate, task, now in [(generation + 1, 'task-one', at(11)),
                                          (generation, 'wrong-task', at(11)),
                                          (generation, 'task-one', claim['deadline'])]:
                with self.subTest(generation=candidate, task=task, now=now), self.assertRaises(ValueError):
                    store.finish(fingerprint, candidate, task, result, now)
            self.assertEqual(store.drafts(), [])
            self.assertIsNone(store.claim(at(1000), TARGET))
            with self.assertRaises(ValueError):
                store.stopped(fingerprint, generation, 'wrong-task')
            store.stopped(fingerprint, generation, 'task-one')
            self.assertNotEqual(store.claim(at(1000), TARGET)['fingerprint'], fingerprint)
            with self.assertRaises(ValueError):
                store.finish(fingerprint, generation, 'task-one', result, at(1001))

    def test_durable_draft_outbox_retains_packet_and_later_occurrences(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
            claim = store.claim(at(10), TARGET)
            store.bind(claim['fingerprint'], claim['generation'], 'task-one')
            with self.assertRaises(ValueError):
                store.finish(claim['fingerprint'], claim['generation'], 'task-one', {'private_payload': 'secret'}, at(12))
            self.assertEqual(store.drafts(), [])
            self.assertEqual(len(store.active()), 1)
            store.finish(claim['fingerprint'], claim['generation'], 'task-one', diagnosis(), at(12))
        with Store(self.path) as store:
            self.ingest(store, [observation('two')])
            self.assertIsNone(store.claim(at(13), TARGET))
            draft = store.drafts()[0]
            self.assertEqual(draft['result'], diagnosis())
            self.assertEqual(draft['packet'], claim['packet'])
            self.assertEqual(draft['delivery_state'], 'pending')
            self.assertEqual(draft['occurrences'], 2)
            self.assertEqual(store.active(), [])

    def test_release_ownership_withdraws_a_saved_draft_without_losing_evidence(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
            claim = store.claim(at(10), TARGET)
            store.bind(claim['fingerprint'], claim['generation'], 'task-one')
            store.finish(claim['fingerprint'], claim['generation'], 'task-one', diagnosis(), at(12))
            self.assertEqual(len(store.drafts()), 1)
            self.ingest(store, [observation('release-event', release_owned=True)])
        with Store(self.path) as store:
            self.assertEqual(store.drafts(), [])
            incident = store.status()['incidents'][0]
            self.assertEqual(incident['release_owned'], 1)
            self.assertEqual(incident['occurrences'], 2)
            self.assertIsNone(store.claim(at(13), TARGET))

    def test_dispatch_target_contract_rejects_invalid_target_before_reservation(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
            for target in ({}, {**TARGET, 'raw': 'private'}, {**TARGET, 'model': ''}, {**TARGET, 'options': []}):
                with self.subTest(target=target), self.assertRaises(ValueError):
                    store.claim(at(10), target)
            self.assertEqual(store.active(), [])
            self.assertIsNotNone(store.claim(at(10), TARGET))

    def test_existing_parent_permissions_are_preserved(self):
        self.path.parent.mkdir(mode=0o755)
        original = stat.S_IMODE(self.path.parent.stat().st_mode)
        with Store(self.path):
            pass
        self.assertEqual(stat.S_IMODE(self.path.parent.stat().st_mode), original)
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)

    def test_release_owned_incidents_are_never_claimed(self):
        with Store(self.path) as store:
            self.ingest(store, [observation(release_owned=True), observation('two', 'other')])
            self.assertEqual(store.claim(at(10), TARGET)['fingerprint'], observation('two', 'other').fingerprint)
            self.assertIsNone(store.claim(at(10), TARGET))

    def test_discovered_release_ownership_fences_an_active_investigation(self):
        with Store(self.path) as store:
            self.ingest(store, [observation()])
            claim = store.claim(at(10), TARGET)
            store.bind(claim['fingerprint'], claim['generation'], 'task-one')
            self.ingest(store, [observation('release-event', release_owned=True)])
            self.assertEqual(store.active()[0]['release_owned'], 1)
            with self.assertRaises(ValueError):
                store.finish(claim['fingerprint'], claim['generation'], 'task-one', diagnosis(), at(12))
            self.assertEqual(store.drafts(), [])
            store.stopped(claim['fingerprint'], claim['generation'], 'task-one')
            self.assertIsNone(store.claim(at(14), TARGET))


if __name__ == '__main__':
    unittest.main()
