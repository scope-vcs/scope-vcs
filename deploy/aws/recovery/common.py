import base64
import hashlib
import json
import os
from pathlib import Path


KEYS = {"SCOPE_OBJECT_ENCRYPTION_KEY", "SCOPE_MEDIA_ENCRYPTION_KEY"}
OPTIONAL_KEYS = {"SCOPE_MEDIA_GRANT_PRIVATE_KEY", "SCOPE_CACHE_GRANT_PRIVATE_KEY"}
PREFIXES = {"objects": "SCOPE_BUCKET", "media": "SCOPE_MEDIA_BUCKET", "cache": "SCOPE_CACHE_BUCKET"}
MAX_BYTES = 10 * 1024**3
MAX_OBJECTS = 100000


class Incomplete(Exception):
    """An incomplete set is never uploaded or reported as recoverable."""


def required(name):
    value = os.environ.get(name, "").strip()
    if not value:
        raise Incomplete("required recovery configuration is missing")
    return value


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def write_json(path, value):
    Path(path).write_bytes(canonical(value))
    Path(path).chmod(0o600)


def keys(path):
    value = json.loads(Path(path).read_bytes())
    if not isinstance(value, dict) or not KEYS <= set(value) or set(value) - KEYS - OPTIONAL_KEYS:
        raise Incomplete("key bundle contains missing or unapproved fields")
    for name in KEYS:
        if not isinstance(value[name], str) or len(base64.b64decode(value[name].strip(), validate=True)) != 32:
            raise Incomplete("data encryption key must decode to 32 bytes")
    for name in OPTIONAL_KEYS & set(value):
        if not isinstance(value[name], str) or "PRIVATE KEY-----" not in value[name]:
            raise Incomplete("signing recovery key is not a PEM private key")
    return value


def object_path(bucket, key):
    if bucket not in PREFIXES or not isinstance(key, str) or not key or len(key.encode()) > 1024:
        raise Incomplete("object identity is invalid")
    return "objects/" + bucket + "/" + hashlib.sha256(key.encode()).hexdigest()
