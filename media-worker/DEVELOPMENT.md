# Media worker development

The worker claims one attachment-processing job at a time, validates the source with
FFmpeg or libheif, writes encrypted derivative chunks, and runs the durable object
cleanup queue in parallel. It exposes `/health` and `/healthz`; readiness requires
the codecs, exact database schema, object storage, and recent polls from both loops.

The production image is built from the repository root because the worker uses
workspace crates:

```sh
docker build -f media-worker/Dockerfile -t scope-media-worker .
docker run --rm --entrypoint scope-media-worker scope-media-worker codec-info
```

The Docker build generates synthetic PNG, JPEG, WebP, animated GIF, HEIC, MP4, MOV,
WebM, rotated, HDR, corrupt, spoofed, and oversized fixtures. It runs the complete
codec pipeline and checks animation, output codecs, rotation, and SDR color tags.
The image retains Debian's codec copyright files under `/usr/share/doc` and copies
`CODEC-NOTICE.md` beside Scope's license and Rust dependency notices.

The service requires `DATABASE_URL`, `SCOPE_MEDIA_BUCKET_NAME`,
`SCOPE_MEDIA_BUCKET_ENDPOINT`, `SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID`,
`SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY`, `SCOPE_MEDIA_BUCKET_REGION`, and a base64
32-byte `SCOPE_MEDIA_ENCRYPTION_KEY`. Local development can set
`SCOPE_MEDIA_OBJECT_STORE=filesystem`. Both the service and worker default to
`data/media-objects` relative to their working directory. Set the same absolute
`SCOPE_MEDIA_OBJECT_STORE_DIR` for both when they run from different directories;
encryption remains mandatory. Mount `/tmp/scope-media-worker` as writable when the root
filesystem is read-only. Startup removes abandoned `job-*` entries and refuses
foreign scratch entries so it never deletes an unowned path.
