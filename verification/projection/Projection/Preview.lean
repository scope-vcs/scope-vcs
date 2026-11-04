import Projection.Policy

inductive PreviewVisibility where
  | allInView
  | someInView
  | noneInView
  deriving DecidableEq, Repr

def previewVisibility (views : Views) (view : String) (c : Commit) : PreviewVisibility :=
  if c.changes.all (fun ch => views.shows view ch.path ch.label) then .allInView
  else if c.changes.all (fun ch => !views.shows view ch.path ch.label) then .noneInView
  else .someInView

def classify (views : Views) (view : String) (commits : List Commit) (id : String) : PreviewVisibility :=
  ((lookup commits id).map (previewVisibility views view)).getD .allInView

structure NativeDetails where
  author : String
  message : String
  changeCount : Nat
  deriving DecidableEq, Repr

structure PreviewFile where
  path : String
  oid : String
  label : String
  deriving DecidableEq, Repr

structure PreviewCommit where
  projectedId : String
  logicalId : String
  parents : List String
  author : Option String
  message : String
  visibility : PreviewVisibility
  changeCount : Nat
  deriving DecidableEq, Repr

structure Summary where
  visibleFiles : Nat
  hiddenFiles : Nat
  visibleCommits : Nat
  hiddenCommits : Nat
  deriving DecidableEq, Repr

structure Preview where
  view : String
  files : List PreviewFile
  commits : List PreviewCommit
  summary : Summary
  deriving DecidableEq, Repr

def projectedTree (commits : List Projected) : Tree :=
  (commits.flatMap (·.changes)).foldl Tree.write []

def previewFiles (oid : String → String) (label : String → String) (commits : List Projected) :
    List PreviewFile :=
  ((projectedTree commits).mergeSort fun a b => !decide (b.path < a.path)).map fun e =>
    ⟨e.path, oid e.blob, label e.path⟩

def previewCommit (visibility : String → PreviewVisibility) (details : String → Option NativeDetails)
    (p : Projected) : Except String PreviewCommit :=
  match p.materialization with
  | .preserve oid parents _ =>
      match details oid with
      | some d => .ok ⟨p.projectedId, p.logicalId, parents, some d.author, d.message,
          visibility p.logicalId, d.changeCount⟩
      | none => .error "native preview metadata is missing"
  | .generate => .ok ⟨p.projectedId, p.logicalId, p.parent.toList, p.author, p.message,
      visibility p.logicalId, p.changes.length⟩

def hiddenCommitCount (full visible : List Projected) : Nat :=
  (full.filter fun p => !(visible.map (·.logicalId)).contains p.logicalId).length

def preview (oid : String → String) (policy : Policy) (commits : List Commit) (sets : List ChangeSet)
    (current : Views) (view : String) (includePrivateCounts : Bool)
    (details : String → Option NativeDetails) : Except String Preview :=
  match fullView current with
  | none => .error "views have no full view"
  | some full => do
      let projection := projectGraph commits sets current view
      let files := previewFiles oid (policy.label full) projection
      let previewCommits ← projection.mapM (previewCommit (classify current view commits) details)
      let hidden :=
        if view != full && includePrivateCounts then
          let fullProjection := projectGraph commits sets current full
          ((previewFiles oid (policy.label full) fullProjection).length - files.length,
            hiddenCommitCount fullProjection projection)
        else (0, 0)
      pure ⟨view, files, previewCommits, ⟨files.length, hidden.1, previewCommits.length, hidden.2⟩⟩

def withoutHiddenCounts (p : Preview) : Preview :=
  { p with summary := { p.summary with hiddenFiles := 0, hiddenCommits := 0 } }
