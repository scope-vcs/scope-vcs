# Check entrypoints

These scripts own the check commands used by `dev/check`, GitHub Actions, and
`.scope/runs/checks.yml`. They run from the repository root, regardless of the
caller's working directory.

| Entrypoint | Coverage |
| --- | --- |
| `backend with-api` | Workspace formatting, tests, API test support, local development helpers, Clippy |
| `backend without-api` | Workspace formatting, tests and Clippy excluding API |
| `cli` | Standalone formatting, tests, distribution selector, Clippy, both release binaries, native installer checks |
| `web` | Tests, types, generated API contract, observer boundary, React health, structure, build |
| `contract` | Generated API TypeScript and validator comparison |
| `policy` | License inventory freshness, complete-tree source size, Rust boundaries, toolchain pins, gate inventory |
| `integration web` | Browser smoke against a running seeded stack |
| `integration cli` | Opt-in two-actor contribution flow against a running seeded stack |
| `ops` | Deployment, staging, benchmark, and AWS infrastructure tests |

Callers install Rust, Node and pnpm dependencies, configure databases and secrets,
and start/stop integration stacks. The contract check needs Rust and web
dependencies. CLI integration requires `SCOPE_API_URL`. The integration entrypoint
explicitly runs the contribution test; ordinary CLI test runs report it as ignored.
GitHub retains native distribution build matrices; these scripts do not select
platforms or provision credentials. Repository policy always checks the full
checkout, including source outside `web/`.

The CLI distribution matrix also runs the portable version and license commands
and the installer check on native Linux, macOS, and Windows runners. It also runs
credential-key, injected Git credential, and browser callback unit tests without
accessing native credential stores. The full
Rust suite runs on Linux; authentication fixtures currently use its file session
store. ARM64 Linux and Windows targets are build-only until native runners are
available. Run the installer check against an existing native build with
`SCOPE_TEST_BINARY=/absolute/path/to/scope node --test cli/distribution/install-smoke.test.mjs`.
The matching `scope-cli-service` binary must be in the same directory.
