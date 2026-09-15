# Check entrypoints

These scripts own the check commands used by `dev/check`, GitHub Actions, and
`.scope/runs/checks.yml`. They run from the repository root, regardless of the
caller's working directory.

| Entrypoint | Coverage |
| --- | --- |
| `backend with-api` | Workspace formatting, tests, API test support, local development helpers, Clippy |
| `backend without-api` | Workspace formatting, tests and Clippy excluding API |
| `cli` | Standalone formatting, tests, distribution selector, Clippy, both release binaries, native installer checks |
| `web` | Tests, types, observer boundary, React health, structure, build |
| `contract` | Generated API TypeScript and validator comparison, owned by the backend gate |
| `policy` | License inventory freshness, complete-tree source size, Rust boundaries, toolchain pins, gate inventory |
| `integration web` | Browser smoke against a running seeded stack |
| `integration cli` | Opt-in two-actor contribution flow against a running seeded stack |
| `ops` | Deployment, staging, benchmark, and AWS infrastructure tests |

Callers install Rust, Node and pnpm dependencies, configure databases and secrets,
and start/stop integration stacks. The contract check needs Rust and web
dependencies. It runs with the backend checks because the API crate generates
the contract, so the web gate does not install Rust. CLI integration requires
`SCOPE_API_URL`. The integration entrypoint explicitly runs the contribution
test; ordinary CLI test runs report it as ignored.
GitHub retains native distribution build matrices; these scripts do not select
platforms or provision credentials. Repository policy always checks the full
checkout, including source outside `web/`.

The CLI distribution matrix also runs the portable version and license commands
and the installer check on native Linux, macOS, and Windows runners. It also runs
credential-key, injected Git credential, and browser callback unit tests without
accessing native credential stores. The full
Rust suite runs on Linux; authentication fixtures currently use its file session
store. All six targets produce complete CLI and analyzer bundles; ARM64 Linux
and Windows remain build-only until native runners are available. Pull requests
build only the three native targets on Blacksmith runners; releases build all
six. Run the
installer check against an existing native build and matching bundle with
`SCOPE_TEST_BINARY=/absolute/path/to/scope SCOPE_TEST_ARTIFACT=/absolute/path/to/scope-target.tar.gz node --test cli/distribution/install-smoke.test.mjs`.
The matching `scope-cli-service` binary must be in the binary's directory.
