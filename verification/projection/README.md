# Projection model

`Projection.lean` models how a view's projection folds a repository's history.
The Rust code it models is `project_graph_after` in
`crates/scope-domain/src/projection.rs`: `ViewFold::project_content`, and
`project_full_graph` for the full view. Lean checks these proofs on every build:

- `project_noninterference`: two histories that a reader cannot tell apart
  produce the same projection for that reader, from any starting fold. The
  `Indistinct` and `LowEq` definitions list exactly what may differ and what
  the projection discloses:
  - a shown commit's logical id;
  - whether it had hidden changes;
  - its author and message only when it had no hidden changes;
  - for a preserved merge into the reader's own view, whose changes the reader
    sees in full, whether it was preserved and its native commits' oids,
    parents and trees.
- `hidden_only_edits_are_indistinct`: two commits with the same id and origin,
  the same visible changes, and the same answer to whether they had hidden
  changes are indistinct, even when their hidden changes differ.

The proofs hold for any visibility function, so they cover the executable
`readerOf` without depending on how it computes view labels. The view rules
themselves are checked by Rust tests, not proved here.

A proof covers the model, not the Rust code. `Main.lean` builds the model as
`projection-model`, which reads histories as JSON and prints their projection.
It does not repeat Rust's validation of views and paths, so it is only fed
histories the Rust side has built. The ignored test
`the_rust_projection_matches_the_proven_lean_model` in
`crates/scope-domain/tests/domain/projection/lean_model.rs` sends 2,000 generated
histories to it and requires the Rust projection to match every field: ids,
parents, author, message, visibility change set, every field of each file's
blob, and each preserved native commit's oid, parents and tree.

Visibility change sets, view transitions and the preview are not modelled yet.
Histories with them are outside both the proofs and the comparison.

Run it with `./dev/check model`, which installs the pinned Lean release from
`dev/tool-versions.json`. CI runs `dev/checks/projection-model` in the Rust
workspace job. When `projection.rs` changes, update the model so the
comparison passes, and update the proofs so the build passes.
