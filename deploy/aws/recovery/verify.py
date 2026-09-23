"""Independent recovery verification of persisted object envelopes and plaintext hashes.

Format: the framed envelope in scope-storage/src/envelope.rs, version 2, used by Git segments and
every other encrypted object. Unknown formats fail closed instead of silently claiming a usable
backup.
"""

import base64
import hashlib
import hmac
import struct
from pathlib import Path

from common import Incomplete, digest, object_path

SEGMENT_LABEL = b"scope-git-segment-v2\0"
OBJECT_LABEL = b"scope-object-v2\0"


def framed_digest(path, key, label, parts, key_id, sinks=()):
    """Decrypts a framed envelope, returning the plaintext SHA-256 and size.

    `parts` are the identity the envelope is bound to: repository and segment IDs for a segment,
    the object key for anything else. Each sink also receives the plaintext.
    """
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
    derived = hmac.new(key, label + b"".join(struct.pack(">Q", len(part)) + part for part in parts), hashlib.sha256).digest()
    cipher = ChaCha20Poly1305(derived)
    identity = b"".join(struct.pack(">I", len(part)) + part for part in parts)
    checksum = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        fixed = stream.read(26)
        if len(fixed) != 26 or fixed[:8] != b"SCGSEG02" or struct.unpack(">I", fixed[8:12])[0] != 2:
            raise Incomplete("encrypted object format is invalid")
        key_size = struct.unpack(">H", fixed[12:14])[0]
        frame_size = struct.unpack(">I", fixed[22:26])[0]
        if not 0 < key_size <= 1024 or not 0 < frame_size <= 16 * 1024**2:
            raise Incomplete("encrypted object bounds are invalid")
        stored_key_id = stream.read(key_size)
        if stored_key_id != key_id:
            raise Incomplete("encrypted object requires an unrecognized escrow key version")
        header = fixed + stored_key_id
        counter = 0
        while True:
            frame = stream.read(9)
            if len(frame) != 9:
                raise Incomplete("encrypted object is truncated")
            actual, length, flags = struct.unpack(">IIB", frame)
            if actual != counter or length > frame_size or flags not in (0, 1) or (flags == 1) != (length == 0):
                raise Incomplete("encrypted object frame is invalid")
            payload = stream.read(length + 16)
            plaintext = cipher.decrypt(fixed[14:22] + struct.pack(">I", counter), payload, header + identity + frame)
            if flags:
                if stream.read(1):
                    raise Incomplete("encrypted object contains trailing bytes")
                break
            checksum.update(plaintext)
            for sink in sinks:
                sink.update(plaintext)
            size += len(plaintext)
            counter += 1
    return checksum.hexdigest(), size


def segment_digest(path, reference, key):
    parts = [reference["repo_id"].encode(), reference["segment_id"].encode()]
    return framed_digest(path, key, SEGMENT_LABEL, parts, b"primary")


def object_digest(path, object_key, key, key_id, sinks=()):
    return framed_digest(path, key, OBJECT_LABEL, [object_key.encode()], key_id, sinks)


def verify(root, inventory, references, escrow):
    root = Path(root)
    for bucket, objects in inventory.items():
        for item in objects:
            path = root / object_path(bucket, item["key"])
            if path.stat().st_size != item["size"] or digest(path) != item["sha256"]:
                raise Incomplete("stored object inventory checksum failed")
    object_key = base64.b64decode(escrow["SCOPE_OBJECT_ENCRYPTION_KEY"].strip())
    media_key = base64.b64decode(escrow["SCOPE_MEDIA_ENCRYPTION_KEY"].strip())
    known = {(bucket, item["key"]) for bucket, values in inventory.items() for item in values}
    manifests = {}
    for ref in sorted(references, key=lambda ref: (ref.get("manifest_id") or "", ref.get("chunk_index") or 0)):
        if (ref["bucket"], ref["key"]) not in known:
            raise Incomplete("database snapshot references a missing object")
        path = root / object_path(ref["bucket"], ref["key"])
        if ref["kind"] == "segment":
            checksum, size = segment_digest(path, ref, object_key)
        elif ref["kind"] == "cache":
            checksum, size = digest(path), path.stat().st_size
        else:
            media = ref["kind"] == "media"
            sinks = []
            if ref.get("manifest_id"):
                manifest = manifests.setdefault(ref["manifest_id"], {"hash": hashlib.sha256(), "size": 0, "sha256": ref["manifest_sha256"], "bytes": ref["manifest_bytes"]})
                sinks.append(manifest["hash"])
            checksum, size = object_digest(path, ref["key"], media_key if media else object_key, b"media" if media else b"primary", sinks)
            if ref.get("manifest_id"):
                manifest["size"] += size
        if checksum != ref["sha256"] or (ref.get("plaintext_bytes") is not None and size != ref["plaintext_bytes"]):
            raise Incomplete("restored plaintext does not match database metadata")
    for manifest in manifests.values():
        if manifest["hash"].hexdigest() != manifest["sha256"] or manifest["size"] != manifest["bytes"]:
            raise Incomplete("restored media manifest checksum failed")
    return {"verified_references": len(references), "verified_media_manifests": len(manifests)}
