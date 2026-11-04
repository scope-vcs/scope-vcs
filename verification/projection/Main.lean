import Lean.Data.Json
import Projection

open Lean

def field (j : Json) (name : String) : Except String Json := j.getObjVal? name

def optString (j : Json) (name : String) : Except String (Option String) := do
  match j.getObjVal? name with
  | .error _ => pure none
  | .ok .null => pure none
  | .ok v => some <$> v.getStr?

def parseChange (j : Json) : Except String Change := do
  pure ⟨← (← field j "path").getStr?, ← (← field j "label").getStr?, ← optString j "content"⟩

def parseNative (j : Json) : Except String Native := do
  let parents ← (← (← field j "parents").getArr?).toList.mapM (·.getStr?)
  pure ⟨← (← field j "oid").getStr?, parents, ← (← field j "tree").getStr?⟩

def parseOrigin (j : Json) : Except String Origin := do
  match ← (← field j "kind").getStr? with
  | "push" => pure .push
  | "merge" =>
      let native ← (← (← field j "native").getArr?).toList.mapM parseNative
      pure (.merge (← (← field j "view").getStr?) (← (← field j "preserve").getBool?) native)
  | other => throw s!"unknown origin {other}"

def parseCommit (j : Json) : Except String Commit := do
  let changes ← (← (← field j "changes").getArr?).toList.mapM parseChange
  pure ⟨← (← field j "id").getStr?, ← parseOrigin (← field j "origin"),
    ← (← field j "author").getStr?, ← (← field j "message").getStr?, changes⟩

def parseView (j : Json) : Except String ViewDef := do
  let includes ← match ← field j "includes" with
    | .null => pure none
    | v => some <$> ((← v.getArr?).toList.mapM (·.getStr?))
  pure ⟨← (← field j "id").getStr?, includes⟩

def optJson : Option String → Json
  | none => .null
  | some value => .str value

def changeJson (c : Change) : Json :=
  Json.mkObj [("path", .str c.path), ("label", .str c.label), ("content", optJson c.content)]

def materializationJson : Materialization → Json
  | .generate => Json.mkObj [("kind", .str "generate")]
  | .preserve oid parents tree =>
      Json.mkObj [("kind", .str "preserve"), ("oid", .str oid),
        ("parents", .arr (parents.map Json.str).toArray), ("tree", .str tree)]

def projectedJson (p : Projected) : Json :=
  Json.mkObj [("projected_id", .str p.projectedId), ("logical_commit_id", .str p.logicalId),
    ("parent", optJson p.parent), ("author", optJson p.author), ("message", .str p.message),
    ("visibility_change_set_id", optJson p.visibilityChangeSet),
    ("changes", .arr (p.changes.map changeJson).toArray),
    ("materialization", materializationJson p.materialization)]

def parseViews (j : Json) : Except String Views := do
  (← j.getArr?).toList.mapM parseView

def parseRelabel (j : Json) : Except String Relabel := do
  pure ⟨← (← field j "path").getStr?, ← (← field j "old").getStr?, ← (← field j "new").getStr?,
    ← optString j "content"⟩

def parseTransition (j : Json) : Except String (Option Transition) := do
  match j with
  | .null => pure none
  | v => pure (some ⟨← parseViews (← field v "before"), ← parseViews (← field v "after")⟩)

def parseSet (j : Json) : Except String ChangeSet := do
  let relabels ← (← (← field j "relabels").getArr?).toList.mapM parseRelabel
  pure ⟨← (← field j "id").getStr?, ← optString j "anchor", ← optString j "source", relabels,
    ← parseTransition (← field j "views")⟩

def viewsJson (views : Views) : Json :=
  .arr (views.map fun v => Json.mkObj [("id", .str v.id),
    ("includes", match v.includes with
      | none => .null
      | some included => .arr (included.map Json.str).toArray)]).toArray

def projectionJson (commits : List Projected) : Json := .arr (commits.map projectedJson).toArray

def runCase (j : Json) : Except String Json := do
  let current ← parseViews (← field j "views")
  let commits ← (← (← field j "commits").getArr?).toList.mapM parseCommit
  let sets ← (← (← field j "sets").getArr?).toList.mapM parseSet
  let reader ← (← field j "reader").getStr?
  let prefixCommits ← (← field j "prefix_commits").getNat?
  let prefixSets ← (← field j "prefix_sets").getNat?
  let start : Cursor := ⟨0, none, initialViews sets current⟩
  let (prefixGraph, suffixGraph) := (commits.take prefixCommits, commits.drop prefixCommits)
  let (prefixChanges, suffixChanges) := (sets.take prefixSets, sets.drop prefixSets)
  let (next, tree) := resume start [] prefixGraph prefixChanges reader
  pure (Json.mkObj [
    ("whole", projectionJson (projectGraph commits sets current reader)),
    ("prefix", projectionJson (projectAfter start [] prefixGraph prefixChanges reader)),
    ("cursor", Json.mkObj [("count", (next.count : Nat)), ("last", optJson next.last),
      ("views", viewsJson next.views)]),
    ("tree", .arr ((tree.mergeSort fun a b => !decide (b.path < a.path)).map fun e => Json.mkObj [("path", .str e.path), ("blob", .str e.blob),
      ("label", .str e.label)]).toArray),
    ("suffix", projectionJson (projectAfter next tree suffixGraph suffixChanges reader))])

def main : IO UInt32 := do
  let input ← (← IO.getStdin).readToEnd
  match Json.parse input >>= (·.getArr?) >>= (·.toList.mapM runCase) with
  | .ok outputs =>
      IO.println (Json.arr outputs.toArray).compress
      pure 0
  | .error message =>
      IO.eprintln message
      pure 1
