from __future__ import annotations

from contextlib import contextmanager
from datetime import datetime, timedelta
import json
import os
from pathlib import Path
import sqlite3
import uuid

from .investigate import validate_result
from .policy import INVESTIGATION_SECONDS, MAX_EVIDENCE, MAX_INVESTIGATORS, Observation, evidence_packet, timestamp

DEGRADED_REASONS = frozenset({'authentication', 'unavailable', 'invalid_response', 'incomplete', 'rate_limited', 'configuration'})


class Store:
    def __init__(self, path):
        self.path = Path(path).expanduser().resolve()
        checkout = Path(__file__).resolve().parents[3]
        if self.path.is_relative_to(checkout):
            raise ValueError('Ledger must live outside the checkout')
        self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        descriptor = os.open(self.path, os.O_CREAT | os.O_RDWR, 0o600)
        os.close(descriptor)
        os.chmod(self.path, 0o600)
        self.connection = sqlite3.connect(self.path, isolation_level=None, timeout=30)
        self.connection.row_factory = sqlite3.Row
        self.connection.execute('PRAGMA foreign_keys = ON')
        self.connection.executescript('''
            CREATE TABLE IF NOT EXISTS sources (
                source_key TEXT PRIMARY KEY, cursor TEXT, status TEXT NOT NULL,
                checked_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS incidents (
                fingerprint TEXT PRIMARY KEY, release_owned INTEGER NOT NULL,
                generation INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE TABLE IF NOT EXISTS observations (
                observation_key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL REFERENCES incidents,
                occurred_at TEXT NOT NULL, record TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS claims (
                fingerprint TEXT PRIMARY KEY REFERENCES incidents, generation INTEGER NOT NULL,
                request_id TEXT NOT NULL UNIQUE, deadline TEXT NOT NULL, task_id TEXT,
                packet TEXT NOT NULL, target TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS drafts (
                fingerprint TEXT PRIMARY KEY REFERENCES incidents, generation INTEGER NOT NULL,
                task_id TEXT NOT NULL, result TEXT NOT NULL, packet TEXT NOT NULL,
                finished_at TEXT NOT NULL, delivery_state TEXT NOT NULL DEFAULT 'pending'
            );
        ''')

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.connection.close()

    @contextmanager
    def transaction(self):
        self.connection.execute('BEGIN IMMEDIATE')
        try:
            yield
            self.connection.execute('COMMIT')
        except BaseException:
            self.connection.execute('ROLLBACK')
            raise

    def cursor(self, source_key):
        row = self.connection.execute('SELECT cursor FROM sources WHERE source_key = ?', (source_key,)).fetchone()
        return row['cursor'] if row else None

    def ingest(self, source_key, start, end, observations: list[Observation], now):
        start, end, now = timestamp(start), timestamp(end), timestamp(now)
        if start > end:
            raise ValueError('Invalid collection interval')
        with self.transaction():
            previous = self.cursor(source_key)
            if previous is not None and start > previous:
                raise ValueError('Collection interval skips the durable cursor')
            for observation in observations:
                if not isinstance(observation, Observation) or not start <= (observation.indexed_at or observation.occurred_at) <= end:
                    raise ValueError('Observation falls outside the complete interval')
                existing = self.connection.execute('SELECT record FROM observations WHERE observation_key = ?', (observation.key,)).fetchone()
                if existing is not None and json.loads(existing['record']) != observation.record():
                    raise ValueError('Observation identity conflicts with durable evidence')
                self.connection.execute('''
                    INSERT INTO incidents (fingerprint, release_owned) VALUES (?, ?)
                    ON CONFLICT(fingerprint) DO UPDATE SET release_owned = MAX(release_owned, excluded.release_owned)
                ''', (observation.fingerprint, int(observation.release_owned)))
                self.connection.execute('''
                    INSERT INTO observations VALUES (?, ?, ?, ?) ON CONFLICT(observation_key) DO NOTHING
                ''', (observation.key, observation.fingerprint, observation.occurred_at, json.dumps(observation.record())))
            self.connection.execute('''
                INSERT INTO sources VALUES (?, ?, 'healthy', ?)
                ON CONFLICT(source_key) DO UPDATE SET cursor = excluded.cursor, status = 'healthy', checked_at = excluded.checked_at
            ''', (source_key, max(previous or end, end), now))

    def degraded(self, source_key, reason, now):
        if reason not in DEGRADED_REASONS:
            raise ValueError('Unknown degraded reason')
        with self.transaction():
            self.connection.execute('''
                INSERT INTO sources VALUES (?, NULL, ?, ?)
                ON CONFLICT(source_key) DO UPDATE SET status = excluded.status, checked_at = excluded.checked_at
            ''', (source_key, reason, timestamp(now)))

    def packet(self, fingerprint):
        observations = [json.loads(row['record']) for row in self.connection.execute(
            'SELECT record FROM observations WHERE fingerprint = ? ORDER BY occurred_at DESC, observation_key LIMIT ?', (fingerprint, MAX_EVIDENCE))]
        return evidence_packet({'fingerprint': fingerprint}, observations)

    def claim(self, now, target: dict):
        if not isinstance(target, dict) or set(target) != {'providerInstanceId', 'model', 'options'}:
            raise ValueError('Invalid investigation target fields')
        if not all(isinstance(target[key], str) and target[key] for key in ('providerInstanceId', 'model')) or not isinstance(target['options'], dict):
            raise ValueError('Invalid investigation target')
        encoded_target = json.dumps(target, allow_nan=False)
        now = timestamp(now)
        with self.transaction():
            if self.connection.execute('SELECT COUNT(*) FROM claims').fetchone()[0] >= MAX_INVESTIGATORS:
                return None
            incident = self.connection.execute('''
                SELECT fingerprint, generation FROM incidents
                WHERE state = 'pending' AND release_owned = 0
                ORDER BY fingerprint LIMIT 1
            ''').fetchone()
            if incident is None:
                return None
            fingerprint, generation = incident['fingerprint'], incident['generation'] + 1
            claim = {'fingerprint': fingerprint, 'generation': generation,
                     'request_id': str(uuid.uuid4()), 'task_id': None,
                     'deadline': timestamp((datetime.fromisoformat(now) + timedelta(seconds=INVESTIGATION_SECONDS)).isoformat()),
                     'packet': self.packet(fingerprint), 'target': json.loads(encoded_target)}
            self.connection.execute('INSERT INTO claims VALUES (?, ?, ?, ?, ?, ?, ?)',
                                    (fingerprint, generation, claim['request_id'], claim['deadline'], None, json.dumps(claim['packet']), encoded_target))
            self.connection.execute("UPDATE incidents SET generation = ?, state = 'active' WHERE fingerprint = ?", (generation, fingerprint))
            return claim

    def bind(self, fingerprint, generation, task_id):
        if not isinstance(task_id, str) or not task_id or len(task_id) > 200:
            raise ValueError('Invalid task ID')
        with self.transaction():
            claim = self.owned(fingerprint, generation)
            if claim['task_id'] not in (None, task_id):
                raise ValueError('Dispatch intent already bound')
            self.connection.execute('UPDATE claims SET task_id = ? WHERE fingerprint = ?', (task_id, fingerprint))

    def owned(self, fingerprint, generation, task_id=None):
        claim = self.connection.execute('SELECT * FROM claims WHERE fingerprint = ? AND generation = ?', (fingerprint, generation)).fetchone()
        if claim is None or (task_id is not None and claim['task_id'] != task_id):
            raise ValueError('Stale investigation ownership')
        return claim

    def active(self):
        return [dict(row) | {'packet': json.loads(row['packet']), 'target': json.loads(row['target'])} for row in self.connection.execute('SELECT claims.*, incidents.release_owned FROM claims JOIN incidents USING(fingerprint) ORDER BY fingerprint')]

    def finish(self, fingerprint, generation, task_id, result: dict, now):
        now = timestamp(now)
        if not isinstance(result, dict) or task_id is None:
            raise ValueError('Invalid draft result')
        with self.transaction():
            claim = self.owned(fingerprint, generation, task_id)
            incident = self.connection.execute('SELECT release_owned FROM incidents WHERE fingerprint = ?', (fingerprint,)).fetchone()
            if incident['release_owned']:
                raise ValueError('Incident belongs to release supervision')
            encoded = json.dumps(validate_result(result, json.loads(claim['packet'])), allow_nan=False)
            if now >= claim['deadline']:
                raise ValueError('Investigation deadline elapsed')
            self.connection.execute('INSERT INTO drafts (fingerprint, generation, task_id, result, packet, finished_at) VALUES (?, ?, ?, ?, ?, ?)',
                                    (fingerprint, generation, task_id, encoded, claim['packet'], now))
            self.connection.execute('DELETE FROM claims WHERE fingerprint = ?', (fingerprint,))
            self.connection.execute("UPDATE incidents SET state = 'draft' WHERE fingerprint = ?", (fingerprint,))

    def stopped(self, fingerprint, generation, task_id):
        if not isinstance(task_id, str) or not task_id:
            raise ValueError('Unbound dispatch intent cannot be released')
        with self.transaction():
            self.owned(fingerprint, generation, task_id)
            self.connection.execute('DELETE FROM claims WHERE fingerprint = ?', (fingerprint,))
            self.connection.execute("UPDATE incidents SET state = 'blocked' WHERE fingerprint = ?", (fingerprint,))

    def confirm_absent(self, fingerprint, generation, request_id):
        with self.transaction():
            claim = self.owned(fingerprint, generation)
            if claim['task_id'] is not None or claim['request_id'] != request_id:
                raise ValueError('Only the confirmed unbound dispatch can be released')
            self.connection.execute('DELETE FROM claims WHERE fingerprint = ?', (fingerprint,))
            self.connection.execute("UPDATE incidents SET state = 'blocked' WHERE fingerprint = ?", (fingerprint,))

    def drafts(self):
        return [dict(row) | {'result': json.loads(row['result']), 'packet': json.loads(row['packet']),
                             'occurrences': row['occurrences']} for row in self.connection.execute('''
            SELECT drafts.*, COUNT(observations.observation_key) AS occurrences, incidents.release_owned
            FROM drafts JOIN incidents USING(fingerprint) JOIN observations USING(fingerprint)
            WHERE incidents.release_owned = 0
            GROUP BY drafts.fingerprint ORDER BY finished_at
        ''')]

    def status(self):
        return {'sources': [dict(row) for row in self.connection.execute('SELECT * FROM sources ORDER BY source_key')],
                'incidents': [dict(row) for row in self.connection.execute('''
                    SELECT incidents.*, COUNT(observation_key) AS occurrences FROM incidents
                    JOIN observations USING(fingerprint) GROUP BY fingerprint ORDER BY fingerprint
                ''')], 'active': self.active(), 'drafts': self.drafts()}
