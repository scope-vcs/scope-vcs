# Architecture

Scope's workspace is organized around durable domain rules, behavior-owned
application use cases, explicit persistence transactions, and thin delivery
layers.

## Dependency direction

The root Cargo workspace contains nineteen packages and excludes `cli/`, which
is a separate workspace. Current `cargo metadata` gives these internal
dependencies:

| Package | Ownership | Direct internal dependencies |
| --- | --- | --- |
| `scope-domain` | Repository, request, projection, reviewed-update, and run rules and persisted shapes | none |
| `scope-cache-domain` | Cache identities, policy, state, and decisions | none |
| `scope-git-process` | Git subprocess lifetime, limits, reaping, and telemetry | none |
| `scope-git-storage` | Bounded, encrypted Git segment ingest, restore, and cleanup across local and remote storage | `scope-domain`, `scope-git-process` |
| `scope-api-contract` | API routes and serialized API/runtime DTOs | `scope-domain` |
| `scope-cache-contract` | Cache grant and endpoint DTOs | `scope-cache-domain` |
| `scope-run-config` | Workflow YAML decoding and compilation | `scope-domain` |
| `scope-object-store` | Object-store interface and filesystem, memory, encrypted, and S3 adapters | `scope-domain` |
| `scope-git` | Projection identity and Git snapshot preparation/materialization | `scope-domain`, optionally `scope-object-store` |
| `scope-postgres` | Metadata stores, migrations, read models, and transaction owners | `scope-domain`, `scope-cache-domain`, `scope-git`, `scope-run-config` |
| `scope-media-storage` | Encrypted, chunked attachment storage and media object manifests | `scope-object-store` |
| `scope-media-service` | Grant-authorized attachment upload and media delivery | `scope-api-contract`, `scope-domain`, `scope-media-storage`, `scope-object-store`, `scope-postgres` |
| `scope-media-worker` | Attachment validation, derivative generation, and media cleanup | `scope-domain`, `scope-media-storage`, `scope-object-store`, `scope-postgres` |
| `scope-content-lifecycle` | Shared source-blob cleanup orchestration | `scope-domain`, `scope-object-store`, `scope-postgres` |
| `api` | Authentication, HTTP/SSE delivery, application use cases, and composition | the contracts, domain crates, orchestration crate, and infrastructure adapters |
| `worker` | Run control, Git compaction, and cleanup polling | domain, API contract, orchestration, and infrastructure adapters |
| `scope-cache-service` | Cache authorization, signed transfers, and reconciliation | cache contract/domain, object store, Postgres |
| `scope-repo-router` | Repository Git/HTTP routing proxy | none |
| `scope-runner-runtime` | Claimed-job setup, cache transfer, process execution, logs, and completion | API/cache contracts and domain crates |

The dependency rules are:

- `scope-domain`, `scope-cache-domain`, `scope-git-process`, and
  `scope-repo-router` are internal leaves. They do not depend on another
  workspace package. `scope-git-storage` depends on the process leaf only for
  the cooperative cancellation signal shared by a Git producer and its
  storage consumer; it does not own subprocess policy.
- Contract crates may depend only on their corresponding domain crate.
- Reusable crates do not depend on application packages, including `api`,
  `worker`, the cache and media services, the media worker, and
  `scope-runner-runtime`.
- Shared application orchestration must have at least two real application
  consumers. `scope-content-lifecycle` is currently consumed by both `api` and
  `worker`.
- Delivery code translates and authorizes. Domain crates decide allowed state;
  Postgres modules own atomic metadata changes; use cases coordinate explicit
  non-database side effects. `scope-git-storage` is the single enforcement
  point for exact plaintext ingest limits, including the first byte beyond the
  accepted boundary, for both API pushes and worker compaction.

`.github/scripts/check-rust-boundaries.mjs` reads both workspaces with
`cargo metadata --locked` to check domain leaves, contract dependencies, and
application boundaries. It also verifies the required behavior-owned source
homes and rejects the retired catch-all module paths.

## Contract and domain boundary

`crates/scope-domain/` owns durable concepts, invariants, transitions, and
persisted shapes. Its public modules are organized by behavior, including
`repository/`, `requests/`, `reviewed_updates/`, and `runs/`. Persisted JSON and
workflow revision identity are locked by `crates/scope-domain/tests/`.

Request operations accept concrete requests, discussions, invitees, and the
facts needed for their decision, such as name collisions or participant counts.
They return mutations and required effects. They do not receive or mutate
repository-wide maps. Postgres loads those facts under the appropriate locks,
checks replay identity, and persists the returned changes.

`crates/scope-api-contract/` owns serialized request/response shapes, runtime
protocol DTOs, and route construction. Public contract fields use wire-owned
types rather than exposing domain enums or structs. Explicit conversions to and
from domain types live in `wire.rs`, `repo_config.rs`, and `runs/`. The shared
`wire_enum!` declaration generates independent wire enums and their explicit
domain conversions, including run states. Repository-run and runner-runtime
payloads live under `runs/`. The compile-time test
`crates/scope-api-contract/tests/wire_boundaries.rs` protects representative
public fields from becoming domain-typed, while contract serialization tests
protect the wire representation.

`scope-api-contract/src/routes.rs` declares each server path once, alongside its
Rust builder and web export name where needed. The declaration generates route
constants, encoded Rust URL builders, and the ordered web route inventory.
`api/src/http/type_exports.rs` consumes that inventory when generating
TypeScript; it does not maintain a second route list.

The dependency from API contract to domain exists for those explicit
conversions. It does not reverse the architectural direction: domain code does
not import API contracts, and transport types do not become the domain model.
The API maps domain and persistence results into contract responses in
`api/src/http/`.

## Application ownership

`api/src/app.rs` composes routes and shared state. `api/src/http/` owns HTTP
extraction, authentication at the delivery edge, status/error mapping, SSE, and
response projection. `api/src/git/` owns local Git and protocol adapters such as
materialization caches, projection repositories, request refs, storage paths,
hooks, and Git subprocess handling.

Cross-system behavior belongs in `api/src/use_cases/`. Its current homes are:

- `request_merge.rs` for preparing, validating, persisting, publishing, and
  cleaning up a request merge;
- `git_receive/` for receive authorization and the separate main-push and
  request-ref completion paths;
- `request_discussion_mutation.rs` for discussion commands, authorization
  context, persistence, result loading, and timeline publication;
- `request_revision_inspection.rs` for shared Git revision membership checks
  and raw diff execution used by review and discussion anchors;
- `run_control.rs` and `run_inspection.rs` for run mutations and authorized run,
  detail, and log reads; and
- `content_cleanup.rs` for repository-storage cleanup and source-blob cleanup
  coordination.

The other applications remain narrow:

- `worker/` runs independent control, compaction, and cleanup roles and pauses
  them when the schema is not ready.
- `cache-service/` verifies cache grants, persists cache metadata, issues signed
  object transfers, and runs reconciliation.
- `media-service/` authorizes upload and media requests with grants and uses
  `scope-media-storage` for encrypted chunk storage.
- `media-worker/` validates uploaded originals, generates derivatives, and
  cleans up media objects.
- `runner-runtime/` converts a claimed wire workflow in `workflow.rs`, uses the
  control-plane client under `api/`, cache behavior under `cache/`, and process
  supervision under `execute/`.
- `web/` consumes generated TypeScript API types and owns browser delivery.

Frontend server data belongs to persistent resource owners under each feature.
`web/src/lib/cached-resource.ts` owns bounded snapshots, subscriptions, and
in-flight requests shared across component mounts. Run history/detail, discussion
timelines and replies, and the paginated request queue reuse those owners. Run
logs retain their own bounded observable state and per-step requests. Keys include the repository, viewer,
and access scope. Relevant changes invalidate resources while preserving valid
data during refresh. Components subscribe and render instead of copying server
state or owning request completion. History and request changes share the
changed-files workbench in `web/src/features/history/changed-files-workbench.tsx`.

## Reading important behavior

Follow these paths from application coordination to durable rules and storage:

| Behavior | Application path | Domain or shared behavior | Persistence and adapters |
| --- | --- | --- | --- |
| Request merge | `api/src/use_cases/request_merge.rs` | `scope-domain/src/reviewed_updates/`, `repository/updates.rs`, request policy | `scope-postgres/src/db/request_merge.rs`, `content_push_transactions.rs`; Git preparation under `api/src/git/` |
| Git receive | `api/src/use_cases/git_receive/` | reviewed-update, repository, request-revision, and workflow-catalog rules in `scope-domain` | `repo_mutation.rs`, `content_push_transactions.rs`, request persistence; upload lifecycle under `api/src/git/import/segment_upload.rs`; bounded ingest under `scope-git-storage` |
| Discussion mutation | `api/src/use_cases/request_discussion_mutation.rs` | `scope-domain/src/requests/discussions.rs` | `scope-postgres/src/db/request_discussions.rs`; HTTP projection in `api/src/http/request_discussions.rs` |
| Run inspection and control | `api/src/use_cases/run_inspection.rs`, `run_control.rs` | `scope-domain/src/runs/` | `run_details.rs`, `run_log_reads.rs`, `runs.rs`, and run-attempt modules; response mapping in `api/src/http/run_*` |
| Content cleanup | `api/src/use_cases/content_cleanup.rs`, `worker/src/cleanup.rs` | `scope-content-lifecycle/src/lib.rs` and `scope-domain::repo_actions` | `scope-postgres/src/db/cleanup_queue/` and `scope-object-store` |

## Persistence transaction ownership

`scope-postgres::db::MetadataStore` exposes behavior-specific `AdminStore`,
`AuthStore`, `CleanupStore`, `CacheStore`, `RepositoryStore`, `RequestStore`,
`MediaStore`, `JobStore`, and `RunStore` handles. Transaction boundaries stay in
`crates/scope-postgres`, not in HTTP handlers.

- `content_push_transactions.rs` persists accepted repository content and
  enqueues projection rebuild and push-trigger work in the same transaction.
- `repo_mutation.rs` owns the reviewed main-push transaction.
- `request_merge.rs` owns the merge transaction, combining accepted content
  with the request lifecycle mutation.
- `request_submission_transactions.rs` owns one-way request submission.
- `request_discussions.rs` owns discussion, reply, transition, and read-state
  transactions.
- `runs.rs`, `run_attempt_mutations.rs`, and
  `run_attempt_persistence.rs` own run and attempt transitions; read models are
  split into `run_details.rs`, `run_log_reads.rs`, and related focused modules.
- `cleanup_queue/` owns queue, claim, revalidation, and completion transactions.

Use cases may prepare Git data or object-store blobs before a database commit.
They carry the relevant content or Git-frontier identity across that work and
explicitly enqueue or perform compensating cleanup when persistence fails.
A successful Postgres transaction remains the authority for metadata state;
filesystem and object operations do not invent domain transitions.

`scope-domain::repository::git::GitHead` owns the current Git coordinates and
its opaque `GitFrontier` digest. Push and merge transactions compare that typed
frontier to reject stale preparation. A frontier is metadata, not a separate
uploaded manifest blob; Git segments and retained run spans determine storage
retention. Cleanup still understands retired manifest references so queued
objects can be removed.

New databases use the current schema baseline. Existing supported databases
adopt it through the maintenance process after schema verification under the
migration lock. The supported starting ledger, writer fence, backup rehearsal,
and recovery steps are documented in
[the schema baseline runbook](operations/current-schema-baseline.md).

## Standalone CLI workspace

`cli/Cargo.toml` and `cli/Cargo.lock` define a one-member workspace excluded
from the root workspace. The CLI depends locally only on `scope-domain` and
`scope-api-contract` and is checked and release-built with its own manifest.

`cli/src/api.rs` owns `ApiSession<'a>`, a borrowed view of the HTTP client, API
base URL, and bearer token. Endpoint functions retain their route, payload, and
error handling; the session applies API authentication once. Commands retain
client lifetime and timeout ownership. Grant-authorized media uploads use their
separate transport rather than inheriting API credentials.

The Railway release job stages `cli/`, `scope-domain`, and
`scope-api-contract` into an upload root without the repository `Cargo.toml`,
then rewrites the two relative dependency paths for that layout. Those two
shared crates therefore keep self-contained package and dependency versions
rather than inheriting root workspace values. The boundary guard verifies both
the one-member CLI metadata graph and the absence of workspace inheritance in
the staged crate manifests.

## Source-size guardrail

`.github/scripts/check-source-size.mjs` enumerates tracked and non-ignored
untracked source with `git ls-files`. It excludes generated TypeScript and
lockfiles, recognizes extensionless scripts in `dev/`, and separates production
from tests and support code.

- Every source at 700 lines or more is reported.
- Production sources at that threshold require a current owner and cohesion
  reason in `.github/source-size-audit.json`.
- Test and support sources are reported separately and are not entered in the
  production ownership ledger.
- Any source above 1000 lines fails, regardless of category.
- Missing, duplicate, miscategorized, or now-small ledger entries fail so the
  ledger cannot become a historical exception list.

The guardrail is enforced in CI and by `./dev/check guardrails`.

## Retained documentation

- `README.md` is the product and repository entrypoint.
- `docs/architecture.md` is the current technical ownership guide.
- `docs/maintenance-cutovers.md` documents migration recovery and the
  forward-only cutover rule; `docs/operations/current-schema-baseline.md` covers
  adoption of the current baseline.
- `deploy/aws/OPERATIONS.md` documents Fargate cloud-run provisioning and
  operation.
- `bench/README.md` documents local Git and deployed-system benchmarks.
- `.scope/RULES.md`, `AGENTS.md`, and `CLAUDE.md` are contribution and agent
  governance, not product architecture references.

Executable help is authoritative for command surfaces: `./dev/scope-dev --help`,
`./dev/check --help`, and `scope-maintenance --help`.
