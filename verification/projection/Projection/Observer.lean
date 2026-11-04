import Projection.Content

structure Observer where
  view : String
  start : Views
  versions : List Views
  secret : String → Bool

def runReader (start : Views) (view : String) (views : Views) : Reader :=
  if fullView start == some view then fullReader view else foldReader view views

namespace Observer

variable (o : Observer)

def reader (views : Views) : Reader := runReader o.start o.view views

def Hidden (path label : String) : Prop := ∀ v ∈ o.versions, v.shows o.view path label = false

def low (path : String) : Bool := !o.secret path

def publicChanges (c : Commit) : List Change := c.changes.filter (o.low ·.path)

def Confined (c : Commit) : Prop := ∀ ch ∈ c.changes, o.secret ch.path → o.Hidden ch.path ch.label

structure SameCommit (a b : Commit) : Prop where
  sourceIdentityIsDisclosed : a.id = b.id
  publicChangesAreDisclosed : o.publicChanges a = o.publicChanges b
  confinedLeft : o.Confined a
  confinedRight : o.Confined b
  observed : ∀ v ∈ o.versions,
    Indistinct (o.reader v) a b ∨ (silent (o.reader v) a ∧ silent (o.reader v) b)

structure UnseenCommit (c : Commit) : Prop where
  noPublicChanges : o.publicChanges c = []
  confined : o.Confined c
  unshown : ∀ v ∈ o.versions, silent (o.reader v) c

inductive Histories (refs : List String) : List Commit → List Commit → Prop where
  | nil : Histories refs [] []
  | unseenLeft {c a b} : o.UnseenCommit c → c.id ∉ refs → Histories refs a b →
      Histories refs (c :: a) b
  | unseenRight {c a b} : o.UnseenCommit c → c.id ∉ refs → Histories refs a b →
      Histories refs a (c :: b)
  | both {c d a b} : o.SameCommit c d → Histories refs a b → Histories refs (c :: a) (d :: b)

def publicRelabels (s : ChangeSet) : List Relabel := s.relabels.filter (o.low ·.path)

def ConfinedSet (s : ChangeSet) : Prop :=
  ∀ rl ∈ s.relabels, o.secret rl.path → o.Hidden rl.path rl.oldLabel ∧ o.Hidden rl.path rl.newLabel

structure SameSet (refs : List String) (a b : ChangeSet) : Prop where
  setIdentityIsDisclosed : a.id = b.id
  anchorIsNamed : a.anchor = b.anchor
  sourceUpdateIsNamed : a.sourceUpdate = b.sourceUpdate
  transitionIsDisclosed : a.views = b.views
  publicRelabelsAreDisclosed : o.publicRelabels a = o.publicRelabels b
  confinedLeft : o.ConfinedSet a
  confinedRight : o.ConfinedSet b
  anchorIsListed : ∀ x, a.anchor = some x → x ∈ refs
  sourceUpdateIsListed : ∀ x, a.sourceUpdate = some x → x ∈ refs
  versionsAreListed : ∀ t, a.views = some t → t.before ∈ o.versions ∧ t.after ∈ o.versions

structure UnseenSet (s : ChangeSet) : Prop where
  noTransition : s.views = none
  noPublicRelabels : o.publicRelabels s = []
  confined : o.ConfinedSet s

inductive SetLists (refs : List String) : List ChangeSet → List ChangeSet → Prop where
  | nil : SetLists refs [] []
  | unseenLeft {s a b} : o.UnseenSet s → SetLists refs a b → SetLists refs (s :: a) b
  | unseenRight {s a b} : o.UnseenSet s → SetLists refs a b → SetLists refs a (s :: b)
  | both {s t a b} : o.SameSet refs s t → SetLists refs a b → SetLists refs (s :: a) (t :: b)

def publicFiles (t : Tree) : Tree := t.filter (o.low ·.path)

def ConfinedTree (t : Tree) : Prop := ∀ e ∈ t, o.secret e.path → o.Hidden e.path e.label

structure SameTree (a b : Tree) : Prop where
  publicFilesAreDisclosed : o.publicFiles a = o.publicFiles b
  confinedLeft : o.ConfinedTree a
  confinedRight : o.ConfinedTree b

end Observer
