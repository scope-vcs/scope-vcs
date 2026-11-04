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
  It does not show that the label is readable in the views version in force
  when that change is emitted.
- `private_control_paths_reach_only_the_full_view`: no change under `/.scope`
  except `/.scope/RULES.md` reaches a reader that is not the full view in any
  views version of the run.
- `hidden_only_edits_are_indistinct`: two commits with the same id and origin,
  the same visible changes, and the same answer to whether they had hidden
  changes are indistinct, even when their hidden changes differ.

## Disclosures

The conditions below say what must be equal between the two histories for the
theorem to apply. They are conservative: the projection reveals at most these
facts, though it does not necessarily reveal every one of them.

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

## Preview

`Projection/Preview.lean` models `projection_preview` in
`crates/scope-domain/src/projection_views.rs`: the files list with its policy
labels, each commit with its parents, author, message, change count and
visibility (`AllInView`, `SomeInView`, `NoneInView`), the summary counts, and
the hidden counts that `include_private_counts` adds.

- `preview_noninterference`: with `include_private_counts` off, two sources a
  reader cannot tell apart give the same preview, including a missing native
  details error. With it on, they give the same preview once
  `withoutHiddenCounts` sets both hidden counts to zero.
- `hidden_counts_need_the_flag_and_a_partial_view`: both hidden counts are zero
  unless `include_private_counts` is set and the reader is not the full view.
  The API sets it only for full-view readers.
- `preview_visibility_is_relative_to_the_reader` and
  `the_full_view_sees_every_commit_in_view`: a commit is `AllInView` when the
  reader sees all its changes, `NoneInView` when it sees none of a non-empty
  set, and `SomeInView` otherwise. The full view sees every commit as
  `AllInView`.
- `a_source_update_carries_its_sources_preview_visibility`: a set commit with a
  source update shows the source commit's visibility.
- `preview_visibility_is_disclosed_only_by_source_updates` and
  `projected_paths_are_public`, the two steps of the main proof: every
  projected commit's visibility agrees between the two sources, and every
  projected path is not secret.

`preview_noninterference` uses the projection's relations and adds these
disclosures:

- the hidden counts, with `include_private_counts` on;
- `sourceUpdateVisibilityIsDisclosed`: the preview visibility of every commit
  that a set names as its source update, which reveals whether that commit had
  hidden changes;
- `policyLabelsOfPublicPathsAreDisclosed`: the policy label of every path that
  is not secret;
- `nativeDetailsOfPreservedCommitsAreDisclosed`: the native details of every
  commit the projection preserves.

It assumes that both views versions are valid (`Views.valid`), that the current
views are one of the run's versions, and `FreshIds`: commit ids are unique and
no set id is a commit id. Without `FreshIds` the preview leaks. It looks up a
commit's visibility by logical id, so a hidden commit with the same id as a
visible one, or with a set's id, changes that commit's visibility. Scope's ids
(`rv_push_*`, `rv_merge_*` and `vchg_*`) never collide this way.

## View rules

`Projection/Model.lean` computes `labels` as a closure that grows by one pass
over includes at a time and stops once a pass adds nothing. It runs at most as
many passes as there are ids it can reach, and `closeWithin_closed` proves that
is enough.

- `labels_are_the_reflexive_transitive_closure_of_includes`;
- `read_access_is_transitive`: a reader that may read a target holds every
  label the target holds;
- `read_access_is_monotone`: for valid views, whatever a target is shown, a
  reader that may read it is shown too;
- `only_the_full_view_reaches_the_full_label`, for valid views;
- `private_control_paths_are_shown_only_to_the_full_view`;
- `readable_views_are_the_readers_labels` and
  `an_anonymous_reader_reads_only_the_anyone_view`, for `readable_by`.

`Views.valid` states the conditions of `Views::new` that these proofs use:
unique ids, `private` as the only full view, and includes that name known views
other than the full view.

## Policy

`Projection/Policy.lean` models `Policy::label` and `Policy::add_rules`. It
splits paths into segments and compares rule paths with the ancestors it builds
from them. The comparison checks that this matches the Rust code's ancestors on
normalized paths, including siblings that share a prefix such as `/a` and
`/ab`.

- `the_nearest_rule_wins`, `a_path_without_a_rule_inherits_its_parent`,
  `the_root_has_only_its_own_rule` and `only_segment_prefixes_apply`, which
  together state that a path's label comes from the rule at its nearest
  ancestor, itself included;
- `the_last_rule_for_a_path_wins`: merging rules keeps the last rule for each
  path;
- `strictAncestors_are_proper_prefixes` and `island_none_iff`: the island
  check visits exactly the proper segment prefixes and finds an island exactly
  when a public rule has a private rule at one;
- `the_public_island_rule_holds` and `a_public_island_is_rejected`: an
  accepted batch leaves no public island in the merged rules, and a rejected
  batch names a public rule and a private proper ancestor of it.

## Not modelled

- `ScopePath::parse`, `Policy::can_read`, `remove_rule`, `split_rules`,
  `join_rules`, policy deserialization, `Views::display_name` and the readers
  and names checks of `Views::new`: tests cover them;
- validation of paths and change sets: the model is only fed inputs the Rust
  side has built.

## Comparison with Rust

A proof covers the model, not the Rust code. `Main.lean` builds the model as
`projection-model`, which reads cases as JSON and prints, for each case, the
whole projection, the projection of a prefix, the cursor and labelled tree
after that prefix, and the projection of the suffix from that cursor and tree.
The ignored test `the_rust_projection_matches_the_proven_lean_model` in
`crates/scope-domain/tests/domain/projection/lean_model.rs` sends 2,000
generated cases to it and requires every field of the Rust output to match,
including each projection's repository id and view key.
Each case also compares both previews, with and without hidden counts.
The cases include anchored, unanchored and dangling sets, source updates,
relabels with and without current content, views transitions, duplicate
commit ids, random policies and missing native details. The test fails if the
generator stops reaching partial and preserved commits, source updates, each
kind of set commit, anchored sets, relabels of a source update's paths,
suffixes resumed after a transition, commits partly or not in view, hidden
counts or missing native details.

`every_small_views_configuration_matches_the_proven_view_rules` enumerates
every configuration `Views::new` accepts with up to two custom views, with and
without the public view. It checks that `Views.valid` holds on each, which shows every configuration
`Views::new` accepts is valid. The converse is not established: `Views.valid`
accepts some configurations `Views::new` rejects, such as a private full view
readable by anyone, and the proofs about valid views also cover those. It compares `labels`,
`may_read`, `readable_by` and `shows` for every reader, label and a public,
private-control and rules path.
`policy_labels_and_the_public_island_rule_match_the_proven_policy` compares
labels before and after `add_rules`, the island it reports and the resulting
policy for 3,000 generated policies and batches.

The build treats warnings as errors, so an unfinished `sorry` fails it.
`Audit.lean` imports `Projection` and `Main` and inspects every constant from
any module whose compiled file was not loaded from the Lean toolchain's own
library directory. A module counts as Lean's by where it was loaded from, not
by its name, so a project helper named under `Std` or `Lake` is still
inspected. It fails:
- if any theorem depends on an axiom other than Lean's standard `propext`,
  `Classical.choice` and `Quot.sound`;
- if any such module declares an `axiom`;
- if one of the theorems listed above is missing or is not a theorem;
- if any definition replaces its compiled code with `implemented_by` or
  `extern`.

So a declared `axiom` cannot stand in for a proof, and the executable the
comparison runs is the definition the theorems are about.

Run it with `./dev/check model`, which installs the pinned Lean release from
`dev/tool-versions.json`. CI runs `dev/checks/projection-model` in the Rust
workspace job. When `projection.rs`, `projection_views.rs`, `views.rs` or
`policy.rs` changes, update the model so the comparison passes, and update the
proofs so the build passes.
