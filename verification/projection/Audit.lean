import Projection
import Lean

open Lean Meta

def standardAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

def provedTheorems : List Name := [``projection_noninterference, ``graph_noninterference,
  ``incremental_noninterference, ``history_noninterference, ``projected_changes_are_readable,
  ``private_control_paths_reach_only_the_full_view, ``hidden_only_edits_are_indistinct]

#eval show MetaM Unit from do
  let env ← getEnv
  let isProjection (name : Name) : Bool :=
    match env.getModuleIdxFor? name with
    | some idx => (`Projection).isPrefixOf (env.header.moduleNames[idx.toNat]!)
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
