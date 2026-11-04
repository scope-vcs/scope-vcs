structure Change where
  path : String
  label : String
  content : Option String
  deriving DecidableEq, Repr

structure Native where
  oid : String
  parents : List String
  tree : String
  deriving DecidableEq, Repr

inductive Materialization where
  | generate
  | preserve (oid : String) (parents : List String) (tree : String)
  deriving DecidableEq, Repr

inductive Origin where
  | push
  | merge (view : String) (preserve : Bool) (native : List Native)
  deriving DecidableEq, Repr

structure Commit where
  id : String
  origin : Origin
  author : String
  message : String
  changes : List Change
  deriving Repr

structure Projected where
  projectedId : String
  logicalId : String
  parent : Option String
  author : Option String
  message : String
  visibilityChangeSet : Option String
  changes : List Change
  materialization : Materialization
  deriving DecidableEq, Repr

structure Fold where
  commits : List Projected
  lastVisible : Option String
  deriving DecidableEq, Repr

structure Reader where
  shows : String → String → Bool
  view : String
  isFull : Bool

def visible (r : Reader) (c : Commit) : List Change :=
  c.changes.filter (fun ch => r.shows ch.path ch.label)

def partialCommit (r : Reader) (c : Commit) : Bool :=
  (visible r c).length < c.changes.length

def preservedNative (r : Reader) (c : Commit) : Option (List Native) :=
  if r.isFull then none
  else match c.origin with
    | .merge view true native =>
        if native ≠ [] ∧ partialCommit r c = false ∧ view = r.view then some native else none
    | _ => none

def preservedCommits (c : Commit) (vis : List Change) : List Native → List Projected
  | [] => []
  | [n] => [⟨n.oid, c.id, n.parents.head?, none, c.message, none, vis, .preserve n.oid n.parents n.tree⟩]
  | n :: rest =>
      ⟨n.oid, c.id, n.parents.head?, none, "Preserved request commit", none, [], .preserve n.oid n.parents n.tree⟩
        :: preservedCommits c vis rest

def lastOid : List Native → Option String
  | [] => none
  | [n] => some n.oid
  | _ :: rest => lastOid rest

def step (r : Reader) (mint : String → Nat → String) (s : Fold) (c : Commit) : Fold :=
  match preservedNative r c with
  | some native =>
      { commits := s.commits ++ preservedCommits c (visible r c) native, lastVisible := lastOid native }
  | none =>
      if visible r c = [] then s
      else
        let pid := mint c.id s.commits.length
        let isPartial := partialCommit r c
        { commits := s.commits ++ [⟨pid, c.id, s.lastVisible,
            if isPartial then none else some c.author,
            if isPartial then "Projected public update" else c.message,
            none, visible r c, .generate⟩],
          lastVisible := some pid }

def project (r : Reader) (mint : String → Nat → String) (s : Fold) (history : List Commit) : Fold :=
  history.foldl (step r mint) s

structure ViewDef where
  id : String
  includes : Option (List String)
  anyone : Bool
  deriving Repr

abbrev Views := List ViewDef

def ViewDef.direct (views : Views) (v : ViewDef) : List String :=
  match v.includes with
  | none => views.map (·.id)
  | some included => included

def includesOf (views : Views) (id : String) : List String :=
  match views.find? (·.id == id) with
  | some v => ViewDef.direct views v
  | none => []

def insertNew (seen : List String) (x : String) : List String :=
  if seen.contains x then seen else seen ++ [x]

def grow (views : Views) (seen : List String) : List String :=
  (seen.flatMap (includesOf views)).foldl insertNew seen

def closeWithin (views : Views) : Nat → List String → List String
  | 0, seen => seen
  | n + 1, seen =>
      if (grow views seen).length = seen.length then seen else closeWithin views n (grow views seen)

def mentioned (views : Views) (view : String) : List String :=
  view :: views.flatMap fun v => v.id :: v.includes.getD []

def labels (views : Views) (view : String) : List String :=
  closeWithin views (mentioned views view).length [view]

def isPrivateControlPath (path : String) : Bool :=
  (path == "/.scope" || path.startsWith "/.scope/") && path != "/.scope/RULES.md"

def isRepoControlPath (path : String) : Bool :=
  path == "/.scope" || path.startsWith "/.scope/"

def fullView (views : Views) : Option String :=
  (views.find? (·.includes.isNone)).map (·.id)

def Views.shows (views : Views) (view path label : String) : Bool :=
  fullView views == some view || ((labels views view).contains label && !isPrivateControlPath path)

def Views.mayRead (views : Views) (reader target : String) : Bool :=
  (labels views reader).contains target

def Views.readableBy (views : Views) : Option String → List String
  | none => ((views.find? (·.anyone)).map (·.id)).toList
  | some reader => (views.map (·.id)).filter ((labels views reader).contains ·)

def Views.valid (views : Views) : Bool :=
  decide (views.map (·.id)).Nodup &&
    (views.filter (·.includes.isNone)).map (·.id) == ["private"] &&
    views.all fun v => match v.includes with
      | none => true
      | some included => !included.contains "private" && included.all ((views.map (·.id)).contains ·)

def foldReader (view : String) (views : Views) : Reader := ⟨views.shows view, view, false⟩

def fullReader (view : String) : Reader := ⟨fun _ _ => true, view, true⟩

def projectedId (view source : String) (sequence : Nat) : String :=
  s!"pv_{view}_{source}_{sequence + 1}"

structure Relabel where
  path : String
  oldLabel : String
  newLabel : String
  current : Option String

structure Transition where
  before : Views
  after : Views

structure ChangeSet where
  id : String
  anchor : Option String
  sourceUpdate : Option String
  relabels : List Relabel
  views : Option Transition

inductive FoldStep where
  | commit (c : Commit)
  | set (s : ChangeSet)

def FoldStep.transition : FoldStep → Option Transition
  | .set s => s.views
  | .commit _ => none

def anchoredIn (commits : List Commit) (s : ChangeSet) : Bool :=
  match s.anchor with
  | some anchor => commits.any (·.id == anchor)
  | none => false

def anchoredAfter (sets : List ChangeSet) (seen : List String) (id : String) : List ChangeSet :=
  if seen.contains id then [] else sets.filter (·.anchor == some id)

def commitSteps (sets : List ChangeSet) : List Commit → List String → List FoldStep
  | [], _ => []
  | c :: rest, seen =>
      .commit c :: (anchoredAfter sets seen c.id).map .set ++ commitSteps sets rest (c.id :: seen)

def foldSteps (commits : List Commit) (sets : List ChangeSet) : List FoldStep :=
  (sets.filter (!anchoredIn commits ·)).map .set ++ commitSteps sets commits []

def lookup : List Commit → String → Option Commit
  | [], _ => none
  | c :: rest, id =>
      match lookup rest id with
      | some d => some d
      | none => if c.id == id then some c else none

structure Entry where
  path : String
  blob : String
  label : String
  deriving DecidableEq, Repr

abbrev Tree := List Entry

def Tree.write (t : Tree) (ch : Change) : Tree :=
  let rest := t.filter (·.path != ch.path)
  match ch.content with
  | some blob => ⟨ch.path, blob, ch.label⟩ :: rest
  | none => rest

def Tree.applyCommit (t : Tree) (c : Commit) : Tree := c.changes.foldl Tree.write t

def Tree.relabelOne (t : Tree) (rl : Relabel) : Tree :=
  t.map (fun e => if e.path == rl.path then { e with label := rl.newLabel } else e)

def Tree.relabel (t : Tree) (s : ChangeSet) : Tree := s.relabels.foldl Tree.relabelOne t

def byPath (a b : Change) : Bool := !decide (b.path < a.path)

structure Viewer where
  view : String
  base : Nat

def Viewer.mint (r : Viewer) (source : String) (appended : Nat) : String :=
  projectedId r.view source (r.base + appended)

def sourceShows (shows : String → String → Bool) (source : Option Commit) (path : String) : Bool :=
  source.any (fun c => c.changes.any (fun ch => ch.path == path && shows ch.path ch.label))

def relabelEmit (before after : String → String → Bool) (source : Option Commit) (rl : Relabel) :
    Option Change :=
  if isRepoControlPath rl.path then none
  else match before rl.path rl.oldLabel, after rl.path rl.newLabel with
    | false, true =>
        if sourceShows before source rl.path then none
        else rl.current.map (fun blob => ⟨rl.path, rl.newLabel, some blob⟩)
    | true, false => some ⟨rl.path, rl.oldLabel, none⟩
    | _, _ => none

def treeEmit (before after : String → String → Bool) (relabels : List Relabel) (e : Entry) :
    Option Change :=
  if relabels.any (·.path == e.path) then none
  else match before e.path e.label, after e.path e.label with
    | false, true => some ⟨e.path, e.label, some e.blob⟩
    | true, false => some ⟨e.path, e.label, none⟩
    | _, _ => none

def setMessage (s : ChangeSet) (source : Option Commit) (changes : List Change) : String :=
  if s.views.isSome then "Projection view boundary"
  else if source.isSome then "Projected public update"
  else if changes.all (·.content.isSome) then "Projection baseline"
  else "Projection visibility boundary"

structure State where
  fold : Fold
  views : Views
  tree : Tree

def State.transition (st : State) (s : ChangeSet) : Views × Views :=
  match s.views with
  | some t => (t.before, t.after)
  | none => (st.views, st.views)

def setChanges (view : String) (st : State) (source : Option Commit) (s : ChangeSet) : List Change :=
  let before := (st.transition s).1.shows view
  let after := (st.transition s).2.shows view
  s.relabels.filterMap (relabelEmit before after source) ++
    if s.views.isSome then (st.tree.filterMap (treeEmit before after s.relabels)).mergeSort byPath
    else []

def setStep (r : Viewer) (find : String → Option Commit) (st : State) (s : ChangeSet) : State :=
  let source := s.sourceUpdate.bind find
  let changes := setChanges r.view st source s
  { fold :=
      if changes = [] then st.fold
      else
        let pid := r.mint s.id st.fold.commits.length
        { commits := st.fold.commits ++ [⟨pid, (source.map (·.id)).getD s.id, st.fold.lastVisible,
            none, setMessage s source changes, some s.id, changes, .generate⟩],
          lastVisible := some pid },
    views := (st.transition s).2,
    tree := st.tree.relabel s }

def commitStep (r : Viewer) (st : State) (c : Commit) : State :=
  { st with fold := step (foldReader r.view st.views) r.mint st.fold c, tree := st.tree.applyCommit c }

def foldStep (r : Viewer) (find : String → Option Commit) (st : State) : FoldStep → State
  | .commit c => commitStep r st c
  | .set s => setStep r find st s

structure Cursor where
  count : Nat
  last : Option String
  views : Views

def projectAfter (cursor : Cursor) (tree : Tree) (commits : List Commit) (sets : List ChangeSet)
    (view : String) : List Projected :=
  if fullView cursor.views == some view then
    (project (fullReader view) (Viewer.mint ⟨view, cursor.count⟩) ⟨[], cursor.last⟩ commits).commits
  else
    ((foldSteps commits sets).foldl (foldStep ⟨view, cursor.count⟩ (lookup commits))
      ⟨⟨[], cursor.last⟩, cursor.views, tree⟩).fold.commits

def initialViews (sets : List ChangeSet) (current : Views) : Views :=
  ((sets.findSome? (·.views)).map (·.before)).getD current

def projectGraph (commits : List Commit) (sets : List ChangeSet) (current : Views) (view : String) :
    List Projected :=
  projectAfter ⟨0, none, initialViews sets current⟩ [] commits sets view

def Cursor.advance (cursor : Cursor) (projected : List Projected) (commits : List Commit)
    (sets : List ChangeSet) : Cursor where
  count := cursor.count + projected.length
  last := (projected.getLast?.map (·.projectedId)).or cursor.last
  views := (((foldSteps commits sets).reverse.findSome? FoldStep.transition).map (·.after)).getD
    cursor.views

def Tree.step (t : Tree) : FoldStep → Tree
  | .commit c => t.applyCommit c
  | .set s => t.relabel s

def Tree.fold (t : Tree) (commits : List Commit) (sets : List ChangeSet) : Tree :=
  (foldSteps commits sets).foldl Tree.step t

def resume (cursor : Cursor) (tree : Tree) (commits : List Commit) (sets : List ChangeSet)
    (view : String) : Cursor × Tree :=
  (cursor.advance (projectAfter cursor tree commits sets view) commits sets, tree.fold commits sets)
