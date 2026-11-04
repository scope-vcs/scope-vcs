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
    ("changes", .arr (p.changes.map changeJson).toArray),
    ("materialization", materializationJson p.materialization)]

def runCase (j : Json) : Except String Json := do
  let views ← (← (← field j "views").getArr?).toList.mapM parseView
  let history ← (← (← field j "commits").getArr?).toList.mapM parseCommit
  let reader ← (← field j "reader").getStr?
  pure (.arr ((projectFor views reader history).map projectedJson).toArray)

def main : IO UInt32 := do
  let input ← (← IO.getStdin).readToEnd
  match Json.parse input >>= (·.getArr?) >>= (·.toList.mapM runCase) with
  | .ok outputs =>
      IO.println (Json.arr outputs.toArray).compress
      pure 0
  | .error message =>
      IO.eprintln message
      pure 1
