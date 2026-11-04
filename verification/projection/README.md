# Projection model

`Projection/Model.lean` models how a view's projection folds a repository's
history and its visibility change sets. The Rust code it models is
`project_graph_after` in `crates/scope-domain/src/projection.rs` and
`projection/labelled_tree.rs`: `fold_steps`, `ViewFold::project_content`,
`ViewFold::set`, the labelled tree kept across commits, `project_full_graph`,
`initial_views`, `ProjectionCursor::advanced` and `LabelledTree::fold`.

## Theorems

Lean checks these proofs on every build:

- `projection_noninterference`: from any cursor, two histories and change-set
  lists that a reader cannot tell apart across every views version produce the
  same projection. `Observer` in `Projection/Observer.lean` fixes the reader's
  view, the run's starting views, every views version the run can be in, and
  which paths are secret. `Histories`, `SetLists` and `SameTree` say what may
  differ.
- `graph_noninterference`: the same for `project_graph`, which starts from
  `initial_views` and an empty tree.
- `incremental_noninterference`: projecting a prefix, advancing the cursor and
  folding the tree, then projecting the suffix from there, gives the same
  suffix projection for two prefixes and suffixes a reader cannot tell apart.
- `history_noninterference`: without change sets, the stronger commit-only
  property holds: a commit's hidden changes may sit on any path. With change
  sets, a hidden write to a path that later becomes readable can be revealed by
  a views transition, so `projection_noninterference` confines hidden changes
  to secret paths.
- `projected_changes_are_readable`: every change in a projection has a path
  and label that the reader can read in at least one views version of the run.
- `private_control_paths_reach_only_the_full_view`: no change under `/.scope`
  except `/.scope/RULES.md` reaches a reader that is not the full view in any
  views version of the run.
- `hidden_only_edits_are_indistinct`: two commits with the same id and origin,
  the same visible changes, and the same answer to whether they had hidden
  changes are indistinct, even when their hidden changes differ.

## Disclosures

The projection discloses exactly what these conditions require to be equal.

For a commit both histories contain (`SameCommit`):

- `sourceIdentityIsDisclosed`: its logical id;
- `publicChangesAreDisclosed`: its changes to paths that are not secret, with
  any label;
- for every views version where the reader sees it (`Indistinct`):
  - `hiddenChangePresenceIsDisclosed`: whether it had hidden changes;
  - its message, and its author, only when it had none;
  - `preservedMergeIsDisclosed`: for a preserved merge into the reader's own
    view, whose changes the reader sees in full, whether it was preserved and
    its native commits' oids, parents and trees.

Changes to secret paths must have labels hidden in every views version
(`Confined`). A commit only one history contains (`UnseenCommit`) has no
changes to paths that are not secret, is shown in no views version, and is
neither an anchor nor a source update of any set (`refs`).

For a change set both lists contain (`SameSet`):

- `setIdentityIsDisclosed`: its id;
- `anchorIsNamed`: its anchor;
- `sourceUpdateIsNamed`: its source update;
- `transitionIsDisclosed`: its views transition;
- `publicRelabelsAreDisclosed`: its relabels of paths that are not secret,
  including their current content.

Relabels of secret paths keep both labels hidden in every views version. A set
only one list contains (`UnseenSet`) has no views transition and relabels only
secret paths. For the labelled tree, `SameTree` discloses every file on a path
that is not secret.

The preview (`projection_views.rs`) is not modelled. Its disclosures, such as
the hidden file and commit counts and a source update's preview visibility, are
outside these proofs.

## Not modelled

- the preview, the policy and the view rules: the proofs hold for the
  executable `labels` and `Views.shows`, but no theorem states a property of
  view includes, such as transitivity of read access;
- validation of views, paths and change sets: the model is only fed inputs the
  Rust side has built.

## Comparison with Rust

A proof covers the model, not the Rust code. `Main.lean` builds the model as
`projection-model`, which reads cases as JSON and prints, for each case, the
whole projection, the projection of a prefix, the cursor and labelled tree
after that prefix, and the projection of the suffix from that cursor and tree.
The ignored test `the_rust_projection_matches_the_proven_lean_model` in
`crates/scope-domain/tests/domain/projection/lean_model.rs` sends 2,000
generated cases to it and requires every field of the Rust output to match.
The cases include anchored, unanchored and dangling sets, source updates,
relabels with and without current content, views transitions and duplicate
commit ids. The test fails if the generator stops reaching partial and
preserved commits, source updates, each kind of set commit, anchored sets,
relabels of a source update's paths, or suffixes resumed after a transition.

Run it with `./dev/check model`, which installs the pinned Lean release from
`dev/tool-versions.json`. CI runs `dev/checks/projection-model` in the Rust
workspace job. When `projection.rs` changes, update the model so the
comparison passes, and update the proofs so the build passes.
