# Isolation

This page lists every channel through which data could cross between
repositories, or between views of one repository. For each channel it says what
may cross and what stops the rest. The route test
`api/src/workflow_tests/cross_repo_isolation.rs` checks the HTTP surface; the
rest is enforced by the code named below.

## Identity

A repository ID is `owner/name`, lowercased. Each repository also has an
incarnation ID, so deleting and recreating a name yields a new incarnation.
Most per-repository caches key on the incarnation. Run-cache keys and Git
segment namespaces key on the repository ID only; their rows are deleted with
the repository and segment IDs are unique, so a recreated repository cannot
reach old objects.

## HTTP routes

Every route template lives in one inventory, `routes!` in
`crates/scope-api-contract/src/routes.rs`, which emits `ROUTE_TEMPLATES`. The
router in `api/src/app.rs` registers routes only from those constants.

A repository route resolves `owner/name` and the caller's access in one read
(`crates/scope-postgres/src/db/repository_access.rs`). Child rows are then
loaded by ID and compared with that repository, for example a request's
`repo_id`, `Run::belongs_to_repository`, or an attachment's request.

A caller who cannot read a repository gets the same 404 as for a missing one.
Member-only routes check readability before membership: `get_repo_config` and
`require_full_view_member` call `find_read_access` first
(`api/src/use_cases/run_inspection.rs`), and Git transport returns the
missing-repository error before it reports an unreadable view
(`authorized_git_read` in `api/src/git/upload.rs`).

`cross_repo_isolation.rs` calls every template with every method as an
anonymous caller, as another user who owns a different repository, and as a
member with a narrow view. Path parameters carry repository A's real IDs. The
test asserts that:

- outsiders get a refusal status, and the same status and body as for a
  repository that does not exist;
- no response carries repository A's file contents, paths, commit message,
  request title or description, discussion text, attachment name, invite
  email, run log, GitHub repository name, owner email, or incarnation ID;
- the narrow member never sees the paths or contents of files outside its
  view, nor the private request and its data.

Bearer capabilities (invite tokens, device codes, user codes) are filled with
values the caller does not hold. Holding one is the authorization.

## Postgres

There is no row-level security. Isolation depends on application queries.

- Tables with `repo_id` or `repository_id`: repositories' live and projection
  files, history, requests, members, invites, runs, run caches, Git heads,
  segments and pool packs, GitHub connections and runs, and media attachments.
- Tables that reach a repository through a parent: request children through
  `request_id`; run jobs, attempts, steps and logs through `run_id`; media
  children through `attachment_id`; invite links through `invite_id`; GitHub
  job logs through `github_job_id`.
- Global tables: `scope_object_references`, `scope_git_segment_references`,
  and `scope_workflow_revisions` (content-addressed by digest). None of them is
  read by a route without a repository-scoped row first.

The step-log page query in `crates/scope-postgres/src/db/run_log_reads.rs`
joins its page back to `scope_run_logs` on `position` alone. That is correct
only because `position` is a global identity key.

## Object storage

- Source blobs are global and content-addressed: `objects/blobs/{sha256}`,
  `objects/git-bundles/{sha256}`, `git-blobs/{oid}`
  (`crates/scope-storage/src/objects/source_blobs.rs`). Repositories that store
  the same bytes share one object, tracked in `scope_object_references`.
  Routes reach a blob only through a `SourceBlob` held by a row of the
  caller's repository. No route takes a blob hash or oid and reads storage
  with it.
- A put always uploads the bytes and never reports whether the object
  existed. The only route that writes caller bytes into this namespace is a
  manual run upload (`api/src/use_cases/run_control.rs`), which hashes the
  caller's own bytes.
- Git segments live under `git/segments/v{N}/{sha256(repo_id)[..32]}/{id}`
  (`segment_object_key` in `crates/scope-storage/src/lib.rs`). Pool packs are
  read with `WHERE repo_id = $1`.
- Objects are encrypted with keys derived per object key; segments use a scope
  of repository and segment ID (`crates/scope-storage/src/envelope.rs`).
- Run-cache objects are `repos/{repository_id}/objects/sha256/{checksum}`. A
  CHECK constraint on `scope_cache_objects` enforces that shape, and every
  lookup filters on `repository_id`. The "already uploaded" answer and the
  quota therefore only ever describe the caller's repository.
- Media parts are `media/v1/staged/{attachment}/...`
  (`crates/scope-media-storage/src/keys.rs`). Keys carry no repository.
  Access goes through grants (below). Attachments are not deduplicated by
  content.

## Presigned URLs and media grants

- Run-cache URLs are signed by the API for at most 15 minutes or the attempt's
  remaining lease, whichever is shorter, and only for the attempt's repository
  (`api/src/use_cases/run_caches.rs`).
- Media grants are Ed25519 tokens valid for 300 seconds. They bind
  attachment, repository, request, viewer and target
  (`api/src/media_grants.rs`). The media service verifies the grant, re-runs
  viewer authorization, and checks the attachment's repository
  (`media-service/src/handlers.rs`). A media URL is a bearer link for its
  lifetime.

## Local Git caches

- All repositories share one cache root. Each derived repository's directory
  name hashes the repository and incarnation; projection repositories also hash
  the view's label set (`api/src/git/storage.rs`,
  `api/src/git/projection_repo.rs`).
- Projection repositories use alternates that point at the full source objects.
  Upload-pack therefore runs with `allowAnySHA1InWant`,
  `allowReachableSHA1InWant` and `allowTipSHA1InWant` all false, and rejects
  wants not reachable from the view's refs (`api/src/git/upload.rs`).
- Receive-pack staging uses `receive.unpackLimit = 1`. Its alternates point
  only at the same incarnation's objects, so thin packs and delta bases resolve
  against the pushing repository alone (`api/src/git/staging.rs`,
  `api/src/git/request_refs/pool.rs`). Request object pools are attached only
  for pushers who read every request.

## Runners

- Each attempt gets its own bootstrap token and task. Tokens are stored as
  hashes (`api/src/auth/runtime.rs`).
- The source route serves only the run behind the attempt token
  (`api/src/http/runtime_protocol.rs`, `api/src/git/run_source.rs`). An
  uploaded bundle is unpacked with `clone --bare --no-local` into a fresh
  directory, so it cannot borrow objects from another repository.
- Cache identity includes the repository, workflow, job, cache name, mount,
  platform and input digests (`crates/scope-domain/src/runs/cache/identity.rs`).
  It does not include the trigger or view. Request runs and main runs of one
  repository can restore each other's caches. A request run starts on its own
  only for a full-view pusher; otherwise a full-view maintainer approves it.

## Change notifications

All changes travel on one Postgres channel, `scope_repo_changes`, and fan out
in memory per repository ID (`api/src/repo_events.rs`). The SSE handler
re-checks read access for every event and drops events whose repository or
incarnation differs (`api/src/http/repo_events.rs`). Readers without the full
view never get run, dependency or GitHub-run events. Request events in views
they cannot read are dropped. Other events become a version-less `Redacted`
change, so a narrow reader learns that the repository changed, not what.

Run event streams and step logs require the full view.

## Logs and telemetry

Tracing records file paths, not contents, when an authorized reader opens a
file (`api/src/http/repos.rs`). The media service logs the URI path without
the grant query.

## Invites

Only hashes of invite tokens are stored (`api/src/auth/tokens.rs`). Holding a
token shows the owner, repository name, invited email, permissions and expiry
without signing in. Accepting requires a signed-in user whose verified email
matches.

## repo-router

`repo-router` reads `owner/repo` from `/git/{view}/{owner}/{repo}/...`, ignores
the view, and picks a backend. It forwards the request unchanged and holds no
repository state. Authorization happens in the API.

## Views within one repository

- One view, `private`, includes every label. At most one view, `public`, is
  readable by anyone. Views include other views transitively
  (`crates/scope-domain/src/views.rs`).
- A file's label is its deepest matching rule, else the repository default.
  The rules file is always public; control paths are always full-view
  (`crates/scope-domain/src/policy.rs`).
- The owner reads the full view, a member reads the view on their membership,
  and everyone else reads the public view
  (`crates/scope-domain/src/repository/access.rs`).
- Files, tree and history read projection rows filtered by the reader's
  labels. A requested view must pass `can_read_view`; a view that exists but
  cannot be read returns 403, not 404.
- Requests in a non-public view are visible only to maintainers who can read
  that view (`crates/scope-domain/src/requests/access.rs`).
- Git fetch serves canonical objects to the full view and a projection to any
  other view.
- Runs and dependency analysis require the full view.

## GitHub

Webhooks are HMAC-verified and routed by GitHub repository ID; a partial
unique index allows one connected Scope repository per GitHub repository.
Installation tokens cover the whole installation and are cached by
installation ID. They are used only server-side. Private request views are
pushed to GitHub only when the connection allows it, and main publication
requires the full view.
