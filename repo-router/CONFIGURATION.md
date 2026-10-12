# Git router configuration

- `SCOPE_REPO_ROUTER_BACKEND` is required. It is the API's host and port, such as `scope-api.railway.internal:8080`.
- `SCOPE_REPO_ROUTER_READ_REPLICAS` defaults to 1 and must be positive. Upload-pack reads rotate across that many discovered API backends; every other request goes to the primary.
- `PORT` defaults to 8080.

## Request limits

Upload-pack requests reserve one of 4 replay slots before reading their incoming
body. A full set returns HTTP 503 immediately. Slots remain reserved while the
request is forwarded, including connection-failure retries.

- Each slot holds up to 64 MiB. Oversized requests return HTTP 413 before forwarding.
- Incomplete upload-pack bodies return HTTP 408 after 15 seconds.
- Upstream connections time out after 2 seconds and upstream reads after 60 seconds.

These limits apply even with one read replica, preserving the same size validation
before forwarding. Receive-pack requests stream directly to the primary and are
never replayed.
