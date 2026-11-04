import Projection
import Main
import Lean

open Lean Meta

def standardAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

def provedTheorems : List Name := [``projection_noninterference, ``graph_noninterference,
  ``incremental_noninterference, ``history_noninterference, ``projected_changes_are_readable,
  ``private_control_paths_reach_only_the_full_view, ``hidden_only_edits_are_indistinct,
  ``preview_noninterference, ``hidden_counts_need_the_flag_and_a_partial_view,
  ``Observer.preview_visibility_is_disclosed_only_by_source_updates, ``Observer.projected_paths_are_public,
  ``labels_are_the_reflexive_transitive_closure_of_includes, ``read_access_is_transitive,
  ``read_access_is_monotone, ``only_the_full_view_reaches_the_full_label,
  ``private_control_paths_are_shown_only_to_the_full_view, ``readable_views_are_the_readers_labels,
  ``the_nearest_rule_wins, ``a_path_without_a_rule_inherits_its_parent, ``the_root_has_only_its_own_rule,
  ``only_segment_prefixes_apply, ``the_last_rule_for_a_path_wins, ``strictAncestors_are_proper_prefixes,
  ``island_none_iff, ``the_public_island_rule_holds, ``a_public_island_is_rejected,
  ``an_anonymous_reader_reads_only_the_anyone_view, ``preview_visibility_is_relative_to_the_reader,
  ``the_full_view_sees_every_commit_in_view, ``a_source_update_carries_its_sources_preview_visibility]

#eval show MetaM Unit from do
  let env ← getEnv
  let toolchain := (← IO.FS.realPath (← getLibDir (← findSysroot))).components
  let mut audited : NameSet := {}
  for module in env.header.moduleNames do
    let olean := (← IO.FS.realPath (← findOLean module)).components
    unless olean.take toolchain.length == toolchain do
      audited := audited.insert module
  for module in [`Projection, `Main] do
    unless audited.contains module do
      throwError m!"{module} was loaded from the Lean toolchain instead of this project"
  let isProjection (name : Name) : Bool :=
    match env.getModuleIdxFor? name with
    | some idx => audited.contains env.header.moduleNames[idx.toNat]!
    | none => false
  let mut theorems : Nat := 0
  for (name, info) in env.constants.toList do
    unless isProjection name do continue
    if (Compiler.getImplementedBy? env name).isSome || isExtern env name then
      throwError m!"{name} replaces its compiled code, so the executable would not run the proved definition"
    if let .axiomInfo _ := info then
      throwError m!"{name} is declared as an axiom instead of being proved"
    if let .thmInfo _ := info then
      theorems := theorems + 1
      for axiomName in ← collectAxioms name do
        unless standardAxioms.contains axiomName do
          throwError m!"{name} depends on the non-standard axiom {axiomName}"
  for name in provedTheorems do
    unless (env.find? name).any (·.isTheorem) do
      throwError m!"{name} is not a proved theorem"
  logInfo m!"audited {theorems} theorems"
