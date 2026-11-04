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
  mintId : String → Nat → String

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

def step (r : Reader) (s : Fold) (c : Commit) : Fold :=
  match preservedNative r c with
  | some native =>
      { commits := s.commits ++ preservedCommits c (visible r c) native, lastVisible := lastOid native }
  | none =>
      if visible r c = [] then s
      else
        let pid := r.mintId c.id s.commits.length
        let isPartial := partialCommit r c
        { commits := s.commits ++ [⟨pid, c.id, s.lastVisible,
            if isPartial then none else some c.author,
            if isPartial then "Projected public update" else c.message,
            none, visible r c, .generate⟩],
          lastVisible := some pid }

def project (r : Reader) (s : Fold) (history : List Commit) : Fold :=
  history.foldl (step r) s

def silent (r : Reader) (c : Commit) : Prop :=
  preservedNative r c = none ∧ visible r c = []

structure Indistinct (r : Reader) (a b : Commit) : Prop where
  sourceIdentityIsDisclosed : a.id = b.id
  visible : visible r a = visible r b
  hiddenChangePresenceIsDisclosed : partialCommit r a = partialCommit r b
  preservedMergeIsDisclosed : preservedNative r a = preservedNative r b
  message : partialCommit r a = false ∨ preservedNative r a ≠ none → a.message = b.message
  author : partialCommit r a = false → preservedNative r a = none → a.author = b.author

inductive LowEq (r : Reader) : List Commit → List Commit → Prop where
  | nil : LowEq r [] []
  | silentLeft {c a b} : silent r c → LowEq r a b → LowEq r (c :: a) b
  | silentRight {c a b} : silent r c → LowEq r a b → LowEq r a (c :: b)
  | both {c d a b} : Indistinct r c d → LowEq r a b → LowEq r (c :: a) (d :: b)

theorem step_silent (r : Reader) (s : Fold) (c : Commit) (h : silent r c) : step r s c = s := by
  unfold step
  rw [h.1]
  simp [h.2]

theorem preservedCommits_congr (a b : Commit) (vis : List Change) (native : List Native)
    (hid : a.id = b.id) (hmsg : a.message = b.message) :
    preservedCommits a vis native = preservedCommits b vis native := by
  induction native with
  | nil => rfl
  | cons n rest ih =>
    cases rest with
    | nil => simp [preservedCommits, hid, hmsg]
    | cons m more => simp [preservedCommits, hid] at ih ⊢; exact ih

theorem step_indistinct (r : Reader) (s : Fold) (a b : Commit) (h : Indistinct r a b) :
    step r s a = step r s b := by
  unfold step
  rw [← h.preservedMergeIsDisclosed]
  cases hp : preservedNative r a with
  | some native =>
    have hmsg : a.message = b.message := h.message (Or.inr (by simp [hp]))
    simp [h.visible, preservedCommits_congr a b (visible r b) native h.sourceIdentityIsDisclosed hmsg]
  | none =>
    simp only
    rw [h.visible, h.hiddenChangePresenceIsDisclosed]
    by_cases hv : visible r b = []
    · simp [hv]
    · simp only [hv, ite_false]
      cases hq : partialCommit r b with
      | true => simp [h.sourceIdentityIsDisclosed]
      | false =>
        have hpa : partialCommit r a = false := by rw [h.hiddenChangePresenceIsDisclosed]; exact hq
        simp [h.sourceIdentityIsDisclosed, h.message (Or.inl hpa), h.author hpa hp]

theorem project_noninterference (r : Reader) :
    ∀ {a b : List Commit}, LowEq r a b → ∀ s, project r s a = project r s b := by
  intro a b h
  induction h with
  | nil => intro s; rfl
  | silentLeft hc _ ih => intro s; simp [project, List.foldl, step_silent r s _ hc] at ih ⊢; exact ih s
  | silentRight hc _ ih => intro s; simp [project, List.foldl, step_silent r s _ hc] at ih ⊢; exact ih s
  | both hcd _ ih => intro s; simp [project, List.foldl, step_indistinct r s _ _ hcd] at ih ⊢; exact ih _

theorem hidden_only_edits_are_indistinct (r : Reader) (a b : Commit)
    (hid : a.id = b.id) (horigin : a.origin = b.origin)
    (hvis : visible r a = visible r b) (hpart : partialCommit r a = partialCommit r b)
    (hmsg : partialCommit r a = false ∨ preservedNative r a ≠ none → a.message = b.message)
    (hauthor : partialCommit r a = false → preservedNative r a = none → a.author = b.author) :
    Indistinct r a b := by
  refine ⟨hid, hvis, hpart, ?_, hmsg, hauthor⟩
  unfold preservedNative
  rw [horigin, hpart]

structure ViewDef where
  id : String
  includes : Option (List String)
  deriving Repr

def ViewDef.direct (views : List ViewDef) (v : ViewDef) : List String :=
  match v.includes with
  | none => views.map (·.id)
  | some included => included

def labelsWithin (views : List ViewDef) : Nat → List String → List String → List String
  | 0, _, seen => seen
  | _ + 1, [], seen => seen
  | fuel + 1, next :: pending, seen =>
      if seen.contains next then labelsWithin views fuel pending seen
      else
        let more := match views.find? (·.id == next) with
          | some v => ViewDef.direct views v
          | none => []
        labelsWithin views fuel (more ++ pending) (next :: seen)

def labels (views : List ViewDef) (view : String) : List String :=
  labelsWithin views ((views.length + 1) * (views.length + 1) + 1) [view] []

def isPrivateControlPath (path : String) : Bool :=
  (path == "/.scope" || path.startsWith "/.scope/") && path != "/.scope/RULES.md"

def fullView (views : List ViewDef) : Option String :=
  (views.find? (·.includes.isNone)).map (·.id)

def readerOf (views : List ViewDef) (view : String) : Reader where
  shows := fun path label =>
    fullView views == some view || ((labels views view).contains label && !isPrivateControlPath path)
  view := view
  isFull := fullView views == some view
  mintId := fun id appended => s!"pv_{view}_{id}_{appended + 1}"

def projectFor (views : List ViewDef) (view : String) (history : List Commit) : List Projected :=
  (project (readerOf views view) ⟨[], none⟩ history).commits
