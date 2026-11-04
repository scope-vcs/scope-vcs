import Projection.Model

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

theorem step_silent (r : Reader) (mint : String → Nat → String) (s : Fold) (c : Commit)
    (h : silent r c) : step r mint s c = s := by
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

theorem step_indistinct (r : Reader) (mint : String → Nat → String) (s : Fold) (a b : Commit)
    (h : Indistinct r a b) : step r mint s a = step r mint s b := by
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

theorem content_noninterference (r : Reader) (mint : String → Nat → String) :
    ∀ {a b : List Commit}, LowEq r a b → ∀ s, project r mint s a = project r mint s b := by
  intro a b h
  induction h with
  | nil => intro s; rfl
  | silentLeft hc _ ih => intro s; simp [project, List.foldl, step_silent r mint s _ hc] at ih ⊢; exact ih s
  | silentRight hc _ ih => intro s; simp [project, List.foldl, step_silent r mint s _ hc] at ih ⊢; exact ih s
  | both hcd _ ih => intro s; simp [project, List.foldl, step_indistinct r mint s _ _ hcd] at ih ⊢; exact ih _

theorem hidden_only_edits_are_indistinct (r : Reader) (a b : Commit)
    (hid : a.id = b.id) (horigin : a.origin = b.origin)
    (hvis : visible r a = visible r b) (hpart : partialCommit r a = partialCommit r b)
    (hmsg : partialCommit r a = false ∨ preservedNative r a ≠ none → a.message = b.message)
    (hauthor : partialCommit r a = false → preservedNative r a = none → a.author = b.author) :
    Indistinct r a b := by
  refine ⟨hid, hvis, hpart, ?_, hmsg, hauthor⟩
  unfold preservedNative
  rw [horigin, hpart]
