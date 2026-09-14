# Git router request limits

Upload-pack requests reserve a replay slot before reading their incoming body.
A full set returns HTTP 503 immediately. Slots remain reserved while the request
is forwarded, including connection-failure retries.

- `SCOPE_REPO_ROUTER_UPLOAD_PACK_REPLAY_SLOTS` defaults to 4, with a supported range of 1–64.
- `SCOPE_REPO_ROUTER_UPLOAD_PACK_REPLAY_MAX_BYTES` defaults to 64 MiB per slot. Oversized requests return HTTP 413 before forwarding.
- `SCOPE_REPO_ROUTER_INCOMING_BODY_TIMEOUT_MILLIS` defaults to 15000. Incomplete upload-pack bodies return HTTP 408 when that deadline expires.

These limits apply even with one read replica, preserving the same size validation
before forwarding. Receive-pack requests stream directly to the primary and are
never replayed. `SCOPE_REPO_ROUTER_READ_TIMEOUT_MILLIS` remains the separate upstream
read timeout.
