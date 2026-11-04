import Projection.Tree

namespace Observer

variable (o : Observer)

inductive Steps (refs : List String) : List FoldStep → List FoldStep → Prop where
  | nil : Steps refs [] []
  | commitLeft {c a b} : o.UnseenCommit c → Steps refs a b → Steps refs (.commit c :: a) b
  | commitRight {c a b} : o.UnseenCommit c → Steps refs a b → Steps refs a (.commit c :: b)
  | setLeft {s a b} : o.UnseenSet s → Steps refs a b → Steps refs (.set s :: a) b
  | setRight {s a b} : o.UnseenSet s → Steps refs a b → Steps refs a (.set s :: b)
  | commit {c d a b} : o.SameCommit c d → Steps refs a b → Steps refs (.commit c :: a) (.commit d :: b)
  | set {s t a b} : o.SameSet refs s t → Steps refs a b → Steps refs (.set s :: a) (.set t :: b)

variable {o}

theorem Steps.append {refs : List String} {a b c d : List FoldStep} (h : o.Steps refs a b)
    (h' : o.Steps refs c d) : o.Steps refs (a ++ c) (b ++ d) := by
  induction h with
  | nil => exact h'
  | commitLeft hc _ ih => exact .commitLeft hc ih
  | commitRight hc _ ih => exact .commitRight hc ih
  | setLeft hs _ ih => exact .setLeft hs ih
  | setRight hs _ ih => exact .setRight hs ih
  | commit hcd _ ih => exact .commit hcd ih
  | set hst _ ih => exact .set hst ih

theorem Steps.unseenLeft {refs : List String} {a b : List FoldStep} :
    ∀ {l : List ChangeSet}, (∀ s ∈ l, o.UnseenSet s) → o.Steps refs a b →
      o.Steps refs (l.map .set ++ a) b
  | [], _, h => h
  | s :: _, hl, h =>
    .setLeft (hl s List.mem_cons_self) (Steps.unseenLeft (fun x hx => hl x (List.mem_cons_of_mem _ hx)) h)

theorem Steps.unseenRight {refs : List String} {a b : List FoldStep} :
    ∀ {l : List ChangeSet}, (∀ s ∈ l, o.UnseenSet s) → o.Steps refs a b →
      o.Steps refs a (l.map .set ++ b)
  | [], _, h => h
  | s :: _, hl, h =>
    .setRight (hl s List.mem_cons_self) (Steps.unseenRight (fun x hx => hl x (List.mem_cons_of_mem _ hx)) h)

theorem SetLists.steps {refs : List String} {a b : List ChangeSet} (h : o.SetLists refs a b) :
    o.Steps refs (a.map .set) (b.map .set) := by
  induction h with
  | nil => exact .nil
  | unseenLeft hs _ ih => exact .setLeft hs ih
  | unseenRight hs _ ih => exact .setRight hs ih
  | both hst _ ih => exact .set hst ih

theorem SetLists.filter {refs : List String} {pa pb : ChangeSet → Bool}
    (hp : ∀ s t, o.SameSet refs s t → pa s = pb t) {a b : List ChangeSet} (h : o.SetLists refs a b) :
    o.SetLists refs (a.filter pa) (b.filter pb) := by
  induction h with
  | nil => exact .nil
  | @unseenLeft s _ _ hs _ ih =>
    rw [List.filter_cons]; split
    · exact .unseenLeft hs ih
    · exact ih
  | @unseenRight s _ _ hs _ ih =>
    rw [List.filter_cons (p := pb)]; split
    · exact .unseenRight hs ih
    · exact ih
  | @both s t _ _ hst _ ih =>
    rw [List.filter_cons, List.filter_cons, hp s t hst]; split
    · exact .both hst ih
    · exact ih

theorem SetLists.unlisted_left {refs : List String} {a b : List ChangeSet} (h : o.SetLists refs a b)
    {x : String} (hx : x ∉ refs) : ∀ s ∈ a.filter (·.anchor == some x), o.UnseenSet s := by
  induction h with
  | nil => intro s hs; cases hs
  | unseenLeft hs _ ih =>
    intro s' hs'
    rw [List.filter_cons] at hs'
    split at hs'
    · cases List.mem_cons.mp hs' with
      | inl heq => subst heq; exact hs
      | inr hin => exact ih s' hin
    · exact ih s' hs'
  | unseenRight _ _ ih => exact ih
  | @both s _ _ _ hst _ ih =>
    intro s' hs'
    rw [List.filter_cons] at hs'
    split at hs'
    · next hanchor =>
      cases List.mem_cons.mp hs' with
      | inl heq =>
        subst heq
        exact absurd (hst.anchorIsListed x (by simpa using hanchor)) hx
      | inr hin => exact ih s' hin
    · exact ih s' hs'

theorem SetLists.symm {refs : List String} {a b : List ChangeSet} (h : o.SetLists refs a b) :
    o.SetLists refs b a := by
  induction h with
  | nil => exact .nil
  | unseenLeft hs _ ih => exact .unseenRight hs ih
  | unseenRight hs _ ih => exact .unseenLeft hs ih
  | both hst _ ih =>
    refine .both ⟨hst.1.symm, hst.2.symm, hst.3.symm, hst.4.symm, hst.5.symm, hst.7, hst.6,
      ?_, ?_, ?_⟩ ih
    · intro x hx; exact hst.anchorIsListed x (hst.anchorIsNamed ▸ hx)
    · intro x hx; exact hst.sourceUpdateIsListed x (hst.sourceUpdateIsNamed ▸ hx)
    · intro t' ht; exact hst.versionsAreListed t' (hst.transitionIsDisclosed ▸ ht)

theorem Histories.any_id {refs : List String} {a b : List Commit} (h : o.Histories refs a b)
    {x : String} (hx : x ∈ refs) : a.any (·.id == x) = b.any (·.id == x) := by
  induction h with
  | nil => rfl
  | @unseenLeft c _ _ _ hn _ ih =>
    rw [List.any_cons, ih]
    have : ¬ (c.id = x) := fun heq => hn (heq ▸ hx)
    simp [this]
  | @unseenRight c _ _ _ hn _ ih =>
    rw [List.any_cons, ← ih]
    have : ¬ (c.id = x) := fun heq => hn (heq ▸ hx)
    simp [this]
  | both hcd _ ih => rw [List.any_cons, List.any_cons, ih, hcd.sourceIdentityIsDisclosed]

theorem anchoredAfter_steps {refs : List String} {sa sb : List ChangeSet} (hs : o.SetLists refs sa sb)
    {seenA seenB : List String} (hseen : ∀ x ∈ refs, seenA.contains x = seenB.contains x)
    (x : String) {a b : List FoldStep} (h : o.Steps refs a b) :
    o.Steps refs ((anchoredAfter sa seenA x).map .set ++ a) ((anchoredAfter sb seenB x).map .set ++ b) := by
  by_cases hx : x ∈ refs
  · unfold anchoredAfter
    rw [hseen x hx]
    split
    · exact h
    · exact (hs.filter (fun s t hst => by rw [hst.anchorIsNamed])).steps.append h
  · have hl : ∀ s ∈ anchoredAfter sa seenA x, o.UnseenSet s := by
      unfold anchoredAfter; split
      · intro s hs'; cases hs'
      · exact hs.unlisted_left hx
    have hr : ∀ s ∈ anchoredAfter sb seenB x, o.UnseenSet s := by
      unfold anchoredAfter; split
      · intro s hs'; cases hs'
      · exact hs.symm.unlisted_left hx
    exact Steps.unseenLeft hl (Steps.unseenRight hr h)

theorem commitSteps_steps {refs : List String} {sa sb : List ChangeSet} (hs : o.SetLists refs sa sb)
    {a b : List Commit} (h : o.Histories refs a b) :
    ∀ {seenA seenB : List String}, (∀ x ∈ refs, seenA.contains x = seenB.contains x) →
      o.Steps refs (commitSteps sa a seenA) (commitSteps sb b seenB) := by
  induction h with
  | nil => intro _ _ _; exact .nil
  | @unseenLeft c _ _ hc hn _ ih =>
    intro seenA seenB hseen
    have hl : ∀ s ∈ anchoredAfter sa seenA c.id, o.UnseenSet s := by
      unfold anchoredAfter; split
      · intro s hs'; cases hs'
      · exact hs.unlisted_left hn
    refine .commitLeft hc (Steps.unseenLeft hl (ih ?_))
    intro x hx
    have : ¬ (x = c.id) := fun heq => hn (heq ▸ hx)
    rw [List.contains_cons, hseen x hx]
    simp [this]
  | @unseenRight c _ _ hc hn _ ih =>
    intro seenA seenB hseen
    have hr : ∀ s ∈ anchoredAfter sb seenB c.id, o.UnseenSet s := by
      unfold anchoredAfter; split
      · intro s hs'; cases hs'
      · exact hs.symm.unlisted_left hn
    refine .commitRight hc (Steps.unseenRight hr (ih ?_))
    intro x hx
    have : ¬ (x = c.id) := fun heq => hn (heq ▸ hx)
    rw [List.contains_cons, hseen x hx]
    simp [this]
  | @both c d _ _ hcd _ ih =>
    intro seenA seenB hseen
    refine .commit hcd ?_
    rw [← hcd.sourceIdentityIsDisclosed]
    refine anchoredAfter_steps hs hseen c.id (ih ?_)
    intro x hx
    rw [List.contains_cons, List.contains_cons, hseen x hx]

theorem foldSteps_steps {refs : List String} {ca cb : List Commit} {sa sb : List ChangeSet}
    (hc : o.Histories refs ca cb) (hs : o.SetLists refs sa sb) :
    o.Steps refs (foldSteps ca sa) (foldSteps cb sb) := by
  unfold foldSteps
  refine Steps.append (SetLists.steps (hs.filter ?_)) (commitSteps_steps hs hc (fun _ _ => rfl))
  intro s t hst
  unfold anchoredIn
  rw [← hst.anchorIsNamed]
  cases hanchor : s.anchor with
  | none => rfl
  | some x => simp only [hc.any_id (hst.anchorIsListed x hanchor)]

inductive SameLookup : Option Commit → Option Commit → Prop where
  | none : SameLookup none none
  | some {c d} : o.SameCommit c d → SameLookup (some c) (some d)

theorem lookup_cons_of_ne {c : Commit} {rest : List Commit} {x : String} (h : (c.id == x) = false) :
    _root_.lookup (c :: rest) x = _root_.lookup rest x := by
  show (match _root_.lookup rest x with | some d => some d | none => if c.id == x then some c else none) = _
  cases _root_.lookup rest x <;> simp [h]

theorem Histories.lookup {refs : List String} {a b : List Commit} (h : o.Histories refs a b)
    {x : String} (hx : x ∈ refs) : o.SameLookup (lookup a x) (lookup b x) := by
  induction h with
  | nil => exact .none
  | @unseenLeft c _ _ _ hn _ ih =>
    rw [lookup_cons_of_ne (by simp only [beq_eq_false_iff_ne, ne_eq]; exact fun heq => hn (heq ▸ hx))]
    exact ih
  | @unseenRight c _ _ _ hn _ ih =>
    rw [lookup_cons_of_ne (by simp only [beq_eq_false_iff_ne, ne_eq]; exact fun heq => hn (heq ▸ hx))]
    exact ih
  | @both c d a' b' hcd _ ih =>
    show o.SameLookup
      (match _root_.lookup a' x with | some e => some e | none => if c.id == x then some c else none)
      (match _root_.lookup b' x with | some e => some e | none => if d.id == x then some d else none)
    generalize _root_.lookup a' x = la at ih ⊢
    generalize _root_.lookup b' x = lb at ih ⊢
    cases ih with
    | none =>
      rw [← hcd.sourceIdentityIsDisclosed]
      by_cases hcx : (c.id == x) = true
      · simp only [hcx, ite_true]; exact .some hcd
      · simp only [hcx, Bool.false_eq_true, ite_false]; exact .none
    | some h' => exact .some h'

end Observer
