"""Run the reference query against live media and durable cleanup tombstones."""
import base64
import re
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import Incomplete
from snapshot import metadata
from verify import verify


class ReferenceTests(unittest.TestCase):
    def query(self):
        # This query uses portable joins/EXISTS/UNION. Only PostgreSQL's explicit
        # result type casts are removed for the dependency-free SQLite fixture.
        sql = Path(__file__).resolve().parents[1].joinpath('references.sql').read_text()
        sql = re.sub(r'::(?:jsonb|text|bigint|integer)\b', '', sql)
        with sqlite3.connect(':memory:') as database:
            database.row_factory = sqlite3.Row
            database.executescript('''
                CREATE TABLE scope_object_references (object_key TEXT);
                CREATE TABLE scope_git_segment_uploads (object_key TEXT, sha256 TEXT, repo_id TEXT, segment_id TEXT, plaintext_bytes INTEGER, state TEXT);
                CREATE TABLE scope_request_media_manifest_chunks (object_key TEXT, sha256 TEXT, plaintext_size_bytes INTEGER, manifest_id TEXT, chunk_index INTEGER);
                CREATE TABLE scope_request_media_manifests (id TEXT, attachment_id TEXT, sha256 TEXT, size_bytes INTEGER);
                CREATE TABLE scope_request_media_upload_parts (object_key TEXT, sha256 TEXT, plaintext_size_bytes INTEGER, attachment_id TEXT, state TEXT);
                CREATE TABLE scope_request_media_cleanup_jobs (attachment_id TEXT, state TEXT);
                CREATE TABLE scope_cache_objects (object_key TEXT, checksum_sha256 TEXT, size_bytes INTEGER);
            ''')
            for attachment, state in [('live', None), ('queued', 'Queued'), ('leased', 'Leased'), ('completed', 'Completed')]:
                database.execute('INSERT INTO scope_request_media_manifests VALUES (?,?,?,?)', (attachment, attachment, 'digest', 1))
                database.execute('INSERT INTO scope_request_media_manifest_chunks VALUES (?,?,?,?,?)', ('chunk-' + attachment, 'digest', 1, attachment, 0))
                database.execute('INSERT INTO scope_request_media_upload_parts VALUES (?,?,?,?,?)', ('part-' + attachment, 'digest', 1, attachment, 'Stored'))
                if state:
                    database.execute('INSERT INTO scope_request_media_cleanup_jobs VALUES (?,?)', (attachment, state))
            return [dict(row) for row in database.execute(sql)]

    def testTombstonesExcludeBothManifestsAndPartsInEveryCleanupState(self):
        rows = self.query()
        self.assertEqual({row['key'] for row in rows}, {'chunk-live', 'part-live'})
        self.assertEqual(len(rows), 2)

    def testMissingLiveMediaRemainsRequiredAndFailsVerification(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            dump = root / 'database.dump'
            dump.write_bytes(b'fixture')
            refs = metadata(self.query(), 'fixture', dump)['references']
            escrow = {name: base64.b64encode(b'k' * 32).decode() for name in ('SCOPE_OBJECT_ENCRYPTION_KEY', 'SCOPE_MEDIA_ENCRYPTION_KEY')}
            for ref in refs:
                with self.subTest(key=ref['key']), self.assertRaisesRegex(Incomplete, 'references a missing object'):
                    verify(root, {'objects': [], 'media': []}, [ref], escrow)
