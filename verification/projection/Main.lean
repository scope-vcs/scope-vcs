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
  pure ⟨← (← field j "id").getStr?, includes, (← (← field j "readers").getStr?) == "anyone"⟩

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
      | some included => .arr (included.map Json.str).toArray),
    ("readers", .str (if v.anyone then "anyone" else "assigned"))]).toArray

def projectionJson (repo view : String) (commits : List Projected) : Json :=
  Json.mkObj [("repo_id", .str repo), ("view_key", .str view),
    ("commits", .arr (commits.map projectedJson).toArray)]

def oidOf (content : String) : String :=
  match Json.parse content >>= (·.getObjValAs? String "git_oid") with
  | .ok oid => oid
  | .error _ => ""

def parseRule (j : Json) : Except String Rule := do
  pure ⟨← (← field j "path").getStr?, ← (← field j "view").getStr?⟩

def parseRules (j : Json) : Except String (List Rule) := do
  (← j.getArr?).toList.mapM parseRule

def parsePolicy (j : Json) : Except String Policy := do
  pure ⟨← (← field j "default").getStr?, ← parseRules (← field j "rules")⟩

def policyJson (p : Policy) : Json :=
  Json.mkObj [("default", .str p.default), ("rules", .arr (p.rules.map fun r =>
    Json.mkObj [("path", .str r.path), ("view", .str r.view)]).toArray)]

def parseDetails (j : Json) : Except String (String × NativeDetails) := do
  pure (← (← field j "oid").getStr?, ⟨← (← field j "author").getStr?, ← (← field j "message").getStr?,
    ← (← field j "changes").getNat?⟩)

def visibilityJson : PreviewVisibility → Json
  | .allInView => .str "all"
  | .someInView => .str "some"
  | .noneInView => .str "none"

def previewJson (repo : String) (p : Preview) : Json :=
  Json.mkObj [("repo_id", .str repo), ("view", .str p.view),
    ("files", .arr (p.files.map fun f => Json.mkObj [("path", .str f.path), ("oid", .str f.oid),
      ("label", .str f.label)]).toArray),
    ("commits", .arr (p.commits.map fun c => Json.mkObj [("projected_id", .str c.projectedId),
      ("logical_commit_id", .str c.logicalId), ("parents", .arr (c.parents.map Json.str).toArray),
      ("author", optJson c.author), ("message", .str c.message), ("visibility", visibilityJson c.visibility),
      ("change_count", (c.changeCount : Nat))]).toArray),
    ("summary", Json.mkObj [("visible_files", (p.summary.visibleFiles : Nat)),
      ("hidden_files", (p.summary.hiddenFiles : Nat)), ("visible_commits", (p.summary.visibleCommits : Nat)),
      ("hidden_commits", (p.summary.hiddenCommits : Nat))])]

def previewResultJson (repo : String) : Except String Preview → Json
  | .ok p => Json.mkObj [("ok", previewJson repo p)]
  | .error message => Json.mkObj [("error", .str message)]

def sortedJson (items : List String) : Json :=
  .arr ((items.mergeSort fun a b => !decide (b < a)).map Json.str).toArray

def runProjection (j : Json) : Except String Json := do
  let current ← parseViews (← field j "views")
  let commits ← (← (← field j "commits").getArr?).toList.mapM parseCommit
  let sets ← (← (← field j "sets").getArr?).toList.mapM parseSet
  let reader ← (← field j "reader").getStr?
  let repo ← (← field j "repo_id").getStr?
  let prefixCommits ← (← field j "prefix_commits").getNat?
  let prefixSets ← (← field j "prefix_sets").getNat?
  let policy ← parsePolicy (← field j "policy")
  let native ← (← (← field j "native").getArr?).toList.mapM parseDetails
  let details := fun oid => (native.find? (·.1 == oid)).map (·.2)
  let start : Cursor := ⟨0, none, initialViews sets current⟩
  let (prefixGraph, suffixGraph) := (commits.take prefixCommits, commits.drop prefixCommits)
  let (prefixChanges, suffixChanges) := (sets.take prefixSets, sets.drop prefixSets)
  let (next, tree) := resume start [] prefixGraph prefixChanges reader
  pure (Json.mkObj [
    ("whole", projectionJson repo reader (projectGraph commits sets current reader)),
    ("prefix", projectionJson repo reader (projectAfter start [] prefixGraph prefixChanges reader)),
    ("cursor", Json.mkObj [("count", (next.count : Nat)), ("last", optJson next.last),
      ("views", viewsJson next.views)]),
    ("tree", .arr ((tree.mergeSort fun a b => !decide (b.path < a.path)).map fun e => Json.mkObj [("path", .str e.path), ("blob", .str e.blob),
      ("label", .str e.label)]).toArray),
    ("suffix", projectionJson repo reader (projectAfter next tree suffixGraph suffixChanges reader)),
    ("preview", previewResultJson repo (preview oidOf policy commits sets current reader false details)),
    ("preview_with_counts",
      previewResultJson repo (preview oidOf policy commits sets current reader true details))])

def runViews (j : Json) : Except String Json := do
  let views ← parseViews (← field j "views")
  let readers ← (← (← field j "readers").getArr?).toList.mapM (·.getStr?)
  let paths ← (← (← field j "paths").getArr?).toList.mapM (·.getStr?)
  let ids := views.map (·.id)
  pure (Json.mkObj [("valid", .bool views.valid),
    ("anyone", sortedJson (views.readableBy none)),
    ("readers", .arr (readers.map fun r => Json.mkObj [("reader", .str r),
      ("labels", sortedJson (labels views r)),
      ("may_read", .arr (readers.map fun t => .bool (views.mayRead r t)).toArray),
      ("readable_by", .arr ((views.readableBy (some r)).map Json.str).toArray),
      ("shows", .arr (paths.map fun path => .arr (readers.map fun l =>
        .bool (views.shows r path l)).toArray).toArray)]).toArray),
    ("ids", .arr (ids.map Json.str).toArray)])

def runPolicy (j : Json) : Except String Json := do
  let policy ← parsePolicy (← field j "policy")
  let additions ← parseRules (← field j "additions")
  let full ← (← field j "full").getStr?
  let paths ← (← (← field j "paths").getArr?).toList.mapM (·.getStr?)
  let labelsOf := fun (p : Policy) => Json.arr (paths.map fun path => Json.str (p.label full path)).toArray
  let (outcome, after) := match policy.addRules additions with
    | .ok next => (Json.mkObj [("ok", .null)], next)
    | .error (child, parent) => (Json.mkObj [("island", .arr #[.str child, .str parent])], policy)
  pure (Json.mkObj [("before", labelsOf policy), ("outcome", outcome), ("after", policyJson after),
    ("after_labels", labelsOf after)])

def runCase (j : Json) : Except String Json := do
  match ← (← field j "kind").getStr? with
  | "projection" => runProjection j
  | "views" => runViews j
  | "policy" => runPolicy j
  | other => throw s!"unknown case kind {other}"

def main : IO UInt32 := do
  let input ← (← IO.getStdin).readToEnd
  match Json.parse input >>= (·.getArr?) >>= (·.toList.mapM runCase) with
  | .ok outputs =>
      IO.println (Json.arr outputs.toArray).compress
      pure 0
  | .error message =>
      IO.eprintln message
      pure 1
