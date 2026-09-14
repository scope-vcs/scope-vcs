#!/usr/bin/env bash
# Extracts a packaged CLI bundle and runs the Rust integration test against its
# managed analyzer runtime. Extra arguments are passed to cargo test.
set -euo pipefail

: "${SCOPE_ARTIFACT:?Set SCOPE_ARTIFACT to the packaged bundle archive}"

runtime_root="$(mktemp -d)"
trap 'rm -rf "$runtime_root"' EXIT
tar -xzf "$SCOPE_ARTIFACT" -C "$runtime_root"
SCOPE_CLI_RUNTIME_DIR="$runtime_root/scope-runtime" cargo test \
  --manifest-path cli/Cargo.toml --locked "$@" --lib \
  local_dependency_analysis::tests::bundled_analyzer_reports_committed_imports \
  -- --ignored --exact
