import base64
import hashlib
import hmac
import io
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from archive import decrypt, encrypt
from capture import failure_reason
from common import Incomplete, canonical, digest, keys, object_path, write_json
from objects import copy_objects, inventory
from publish import publish, upload_archive
from verify import segment_digest, verify


KEY = b"k" * 32
ESCROW = {"SCOPE_OBJECT_ENCRYPTION_KEY": base64.b64encode(KEY).decode(), "SCOPE_MEDIA_ENCRYPTION_KEY": base64.b64encode(KEY).decode()}


def framed(label, parts, key_id, frames):
    derived = hmac.new(KEY, label + b"".join(struct.pack(">Q", len(part)) + part for part in parts), hashlib.sha256).digest()
    header = b"SCGSEG02" + struct.pack(">IH", 2, len(key_id)) + b"n" * 8 + struct.pack(">I", 1024) + key_id
    identity = b"".join(struct.pack(">I", len(part)) + part for part in parts)
    result = header
    for counter, data in enumerate([*frames, b""]):
        frame = struct.pack(">IIB", counter, len(data), int(not data))
        result += frame + ChaCha20Poly1305(derived).encrypt(b"n" * 8 + struct.pack(">I", counter), data, header + identity + frame)
    return result


def envelope(key, plaintext, key_id=b"primary"):
    frames = [plaintext[offset:offset + 1024] for offset in range(0, len(plaintext), 1024)]
    return framed(b"scope-object-v2\0", [key.encode()], key_id, frames)


def segment(repository, segment_id, frames):
    return framed(b"scope-git-segment-v2\0", [repository.encode(), segment_id.encode()], b"primary", frames)


class Body(io.BytesIO):
    def iter_chunks(self, chunk_size):
        while chunk := self.read(chunk_size):
            yield chunk


class Source:
    def __init__(self, values):
        self.values = values
        self.downloads = 0
        self.change_before_get = False

    def get_paginator(self, name):
        return self

    def paginate(self, **kwargs):
        return [{"Contents": [{"Key": key, "Size": len(value), "ETag": hashlib.md5(value).hexdigest()} for key, value in self.values.items()]}]

    def get_object(self, Key, IfMatch, **kwargs):
        self.downloads += 1
        data = self.values[Key]
        actual = hashlib.md5(data).hexdigest()
        if IfMatch != actual or self.change_before_get:
            raise Incomplete("If-Match failed")
        return {"ContentLength": len(data), "ETag": actual, "Body": Body(data)}


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)

    def tearDown(self):
        self.scratch.cleanup()

    def fixture(self):
        key = "objects/blobs/" + hashlib.sha256(b"repository content").hexdigest()
        source = Source({key: envelope(key, b"repository content")})
        clients = {"objects": (source, "source-bucket"), "media": (Source({}), "media-bucket")}
        before = inventory(clients, 10000, 100)
        stored = copy_objects(clients, before, self.root)
        references = [{"kind": "content", "bucket": "objects", "key": key, "sha256": key.rsplit("/", 1)[1]}]
        return clients, stored, references

    def testExactEncryptedInventoryAndPlaintextCanBeRecovered(self):
        _, stored, refs = self.fixture()
        self.assertEqual(verify(self.root, stored, refs, ESCROW)["verified_references"], 1)

    def testFailureReasonNamesOnlyFixedMessages(self):
        self.assertEqual(failure_reason(Incomplete("source object was truncated")), "source object was truncated")
        self.assertEqual(failure_reason(RuntimeError("https://key:secret@bucket/private-object")), "RuntimeError")

    def testMissingReferencedObjectFailsEvenWhenInventoriesMatch(self):
        _, stored, refs = self.fixture()
        refs[0]["key"] = "objects/blobs/missing"
        with self.assertRaises(Incomplete):
            verify(self.root, stored, refs, ESCROW)

    def testWrongEscrowKeyCannotPassRecovery(self):
        _, stored, refs = self.fixture()
        wrong = dict(ESCROW, SCOPE_OBJECT_ENCRYPTION_KEY=base64.b64encode(b"z" * 32).decode())
        with self.assertRaises(Exception):
            verify(self.root, stored, refs, wrong)

    def testAuthenticatedBytesMustMatchDatabasePlaintextHash(self):
        _, stored, refs = self.fixture()
        refs[0]["sha256"] = "f" * 64
        with self.assertRaises(Incomplete):
            verify(self.root, stored, refs, ESCROW)

    def testInventoryTamperingFails(self):
        _, stored, refs = self.fixture()
        stored["objects"][0]["sha256"] = "f" * 64
        with self.assertRaises(Incomplete):
            verify(self.root, stored, refs, ESCROW)

    def testChangingSourceEtagIsNotCopied(self):
        source = Source({"objects/blobs/key": b"payload"})
        clients = {"objects": (source, "bucket")}
        before = inventory(clients, 1000, 10)
        source.change_before_get = True
        with self.assertRaises(Incomplete):
            copy_objects(clients, before, self.root)

    def testCostCapsFailBeforeDownloading(self):
        source = Source({"one": b"12345", "two": b"67890"})
        with self.assertRaises(Incomplete):
            inventory({"objects": (source, "bucket")}, 9, 10)
        with self.assertRaises(Incomplete):
            inventory({"objects": (source, "bucket")}, 100, 1)
        self.assertEqual(source.downloads, 0)

    def testStoragePathsDoNotFollowSourceKeys(self):
        path = object_path("media", "../../outside")
        self.assertNotIn("..", path)
        self.assertEqual(len(path.split("/")[-1]), 64)

    def testKeyEscrowRefusesStorageCredentials(self):
        path = self.root / "keys.json"
        write_json(path, dict(ESCROW, AWS_SECRET_ACCESS_KEY="never archive this"))
        with self.assertRaises(Incomplete):
            keys(path)

    def testGitSegmentFrameAuthenticationAndPlaintextHash(self):
        path = self.root / "segment"
        path.write_bytes(segment("repo", "segment", [b"PACK", b"content"]))
        self.assertEqual(segment_digest(path, {"repo_id": "repo", "segment_id": "segment"}, KEY), (hashlib.sha256(b"PACKcontent").hexdigest(), 11))
        with self.assertRaises(Exception):
            segment_digest(path, {"repo_id": "other", "segment_id": "segment"}, KEY)
        path.write_bytes(path.read_bytes()[:-1])
        with self.assertRaises(Exception):
            segment_digest(path, {"repo_id": "repo", "segment_id": "segment"}, KEY)

    def testMediaManifestCrossingEightMibChunkBoundary(self):
        chunks = [b"a" * (8 * 1024**2), b"last chunk"]
        whole = b"".join(chunks)
        source = Source({f"media/v1/chunk-{index}": envelope(f"media/v1/chunk-{index}", part, b"media") for index, part in enumerate(chunks, 1)})
        clients = {"media": (source, "bucket")}
        stored = copy_objects(clients, inventory(clients, 10 * 1024**2, 10), self.root)
        refs = [{"kind": "media", "bucket": "media", "key": f"media/v1/chunk-{index}", "sha256": hashlib.sha256(part).hexdigest(), "plaintext_bytes": len(part), "manifest_id": "original", "chunk_index": index, "manifest_sha256": hashlib.sha256(whole).hexdigest(), "manifest_bytes": len(whole)} for index, part in enumerate(chunks, 1)]
        self.assertEqual(verify(self.root, stored, refs, ESCROW)["verified_media_manifests"], 1)

    @unittest.skipUnless(shutil.which("age") and shutil.which("age-keygen"), "age binaries required for encrypted roundtrip")
    def testIsolatedAgeRoundtripPreservesDatabaseObjectsAndKeys(self):
        _, stored, refs = self.fixture()
        (self.root / "database.dump").write_bytes(b"test-only-database-dump")
        write_json(self.root / "keys.json", ESCROW)
        write_json(self.root / "manifest.json", {"inventory": stored, "references": refs})
        with tempfile.TemporaryDirectory() as external:
            identity = Path(external) / "identity"
            subprocess.run(["age-keygen", "--output", str(identity)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            recipient = subprocess.check_output(["age-keygen", "-y", str(identity)], stderr=subprocess.DEVNULL).decode().strip()
            archive = Path(external) / "recovery.age"
            restored = Path(external) / "restored"
            encrypt(self.root, recipient, archive)
            self.assertNotIn(b"SCOPE_OBJECT_ENCRYPTION_KEY", archive.read_bytes())
            decrypt(archive, identity, restored)
            self.assertEqual(digest(restored / "database.dump"), digest(self.root / "database.dump"))
            self.assertEqual(verify(restored, stored, refs, keys(restored / "keys.json"))["verified_references"], 1)
            broken = Path(external) / "broken.age"
            broken.write_bytes(archive.read_bytes()[:-10])
            with self.assertRaises(Exception):
                decrypt(broken, identity, Path(external) / "broken")


class Destination:
    def __init__(self):
        self.calls = []

    def put_object(self, **kwargs):
        self.calls.append(kwargs)
        assert kwargs["ServerSideEncryption"] == "AES256"
        return {"VersionId": str(len(self.calls)), "ChecksumSHA256": kwargs["ChecksumSHA256"]}


class PublishTests(unittest.TestCase):
    def testCompletionMarkerBindsImmutableArchiveVersion(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "archive"
            archive.write_bytes(b"encrypted fixture")
            destination = Destination()
            result = publish(destination, "recovery", "capture", archive, {"object_count": 2})
            marker = json.loads(destination.calls[1]["Body"])
            self.assertEqual(marker["archive_version"], "1")
            self.assertEqual(marker["archive_sha256"], digest(archive))
            self.assertEqual(result["complete_version"], "2")
            self.assertEqual(len(destination.calls), 2)

    def testMultipartPublicationVerifiesEveryPartAndReturnsVersion(self):
        class Multipart:
            def __init__(self):
                self.parts = []
            def create_multipart_upload(self, **kwargs):
                self.encryption = kwargs["ServerSideEncryption"]
                return {"UploadId": "upload"}
            def upload_part(self, **kwargs):
                self.parts.append(kwargs["Body"])
                return {"ETag": str(kwargs["PartNumber"]), "ChecksumSHA256": kwargs["ChecksumSHA256"]}
            def complete_multipart_upload(self, **kwargs):
                self.completed = kwargs["MultipartUpload"]["Parts"]
                return {"VersionId": "immutable-version"}
        with tempfile.TemporaryDirectory() as root, patch("publish.PART_BYTES", 4):
            archive = Path(root) / "archive"
            archive.write_bytes(b"encrypted archive")
            client = Multipart()
            self.assertEqual(upload_archive(client, "bucket", "sets/capture/archive", archive), "immutable-version")
            self.assertEqual(b"".join(client.parts), archive.read_bytes())
            self.assertEqual(len(client.completed), len(client.parts))
            self.assertEqual(client.encryption, "AES256")


if __name__ == "__main__":
    unittest.main()
