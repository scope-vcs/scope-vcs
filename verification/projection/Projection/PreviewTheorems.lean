import Projection.PublicPaths
import Projection.Preview
import Projection.ViewRules

def FreshIds (commits : List Commit) (sets : List ChangeSet) : Prop :=
  (commits.map (·.id)).Nodup ∧ ∀ s ∈ sets, s.id ∉ commits.map (·.id)

theorem lookup_found : ∀ {l : List Commit} {x : String} {c : Commit}, lookup l x = some c → c.id = x
  | [], _, _, h => by cases h
  | d :: rest, x, c, h => by
    unfold lookup at h
    split at h
    · next hr => cases h; exact lookup_found hr
    · split at h
      · next heq => cases h; simpa using heq
      · cases h

theorem lookup_missing : ∀ {l : List Commit} {x : String}, x ∉ l.map (·.id) → lookup l x = none
  | [], _, _ => rfl
  | d :: rest, x, h => by
    have hrest : x ∉ rest.map (·.id) := fun hx => h (List.mem_cons_of_mem _ hx)
    have hd : (d.id == x) = false := by
      simp only [beq_eq_false_iff_ne, ne_eq]
      exact fun heq => h (heq ▸ List.mem_cons_self)
    unfold lookup
    rw [lookup_missing hrest]
    simp [hd]

theorem lookup_unique : ∀ {l : List Commit} {c : Commit}, (l.map (·.id)).Nodup → c ∈ l → lookup l c.id = some c
  | [], _, _, h => by cases h
  | d :: rest, c, hnodup, hc => by
    have hn := List.nodup_cons.mp hnodup
    unfold lookup
    cases List.mem_cons.mp hc with
    | inl heq =>
      subst heq
      rw [lookup_missing hn.1]
      simp
    | inr hin => rw [lookup_unique hn.2 hin]

theorem preview_visibility_is_relative_to_the_reader (views : Views) (view : String) (c : Commit) :
    previewVisibility views view c =
      if partialCommit (foldReader view views) c = false then .allInView
      else if visible (foldReader view views) c = [] then .noneInView else .someInView := by
  have hall : c.changes.all (fun ch => views.shows view ch.path ch.label) =
      !partialCommit (foldReader view views) c := by
    unfold partialCommit visible
    have hle := List.length_filter_le (fun ch => (foldReader view views).shows ch.path ch.label) c.changes
    cases h : c.changes.all (fun ch => views.shows view ch.path ch.label) with
    | true =>
      have := List.length_filter_eq_length_iff.mpr (List.all_eq_true.mp h)
      simp [foldReader] at this ⊢
      omega
    | false =>
      have hne : ¬ (c.changes.filter fun ch => views.shows view ch.path ch.label).length = c.changes.length :=
        fun heq => by rw [List.all_eq_true.mpr (List.length_filter_eq_length_iff.mp heq)] at h; cases h
      simp [foldReader] at hne hle ⊢
      omega
  have hnone : c.changes.all (fun ch => !views.shows view ch.path ch.label) =
      (visible (foldReader view views) c).isEmpty := by
    unfold visible
    cases h : (c.changes.filter fun ch => (foldReader view views).shows ch.path ch.label) with
    | nil =>
      have := List.filter_eq_nil_iff.mp h
      simp only [List.isEmpty_nil]
      exact List.all_eq_true.mpr fun ch hch => by simpa [foldReader] using this ch hch
    | cons x rest =>
      have hx : x ∈ c.changes.filter fun ch => (foldReader view views).shows ch.path ch.label := by
        rw [h]; exact List.mem_cons_self
      simp only [List.isEmpty_cons]
      cases hall' : c.changes.all (fun ch => !views.shows view ch.path ch.label) with
      | false => rfl
      | true =>
        have := List.all_eq_true.mp hall' x (List.mem_filter.mp hx).1
        have := (List.mem_filter.mp hx).2
        simp_all [foldReader]
  unfold previewVisibility
  rw [hall, hnone]
  cases partialCommit (foldReader view views) c <;> cases visible (foldReader view views) c <;> rfl

theorem the_full_view_sees_every_commit_in_view {views : Views} {view : String} (h : fullView views = some view)
    (c : Commit) : previewVisibility views view c = .allInView := by
  unfold previewVisibility
  simp [Views.shows, h]

theorem changes_nil_of_indistinct {r : Reader} {a b : Commit} (h : Indistinct r a b) :
    a.changes = [] ↔ b.changes = [] := by
  have side : ∀ {x y : Commit}, visible r x = visible r y → partialCommit r x = partialCommit r y →
      x.changes = [] → y.changes = [] := by
    intro x y hv hp hx
    have hvx : visible r x = [] := by simp [visible, hx]
    have hpx : partialCommit r x = false := by simp [partialCommit, hvx, hx]
    have hpy := hp ▸ hpx
    have hvy := hv ▸ hvx
    simp only [partialCommit, hvy, List.length_nil, decide_eq_false_iff_not, Nat.not_lt] at hpy
    exact List.eq_nil_of_length_eq_zero (by omega)
  exact ⟨side h.visible h.hiddenChangePresenceIsDisclosed,
    side h.visible.symm h.hiddenChangePresenceIsDisclosed.symm⟩

theorem preservedCommits_logical (c : Commit) (vis : List Change) :
    ∀ (native : List Native), ∀ p ∈ preservedCommits c vis native, p.logicalId = c.id
  | [] => by intro p hp; cases hp
  | [n] => by intro p hp; simp [preservedCommits] at hp; subst hp; rfl
  | n :: m :: rest => by
    intro p hp
    simp only [preservedCommits, List.mem_cons] at hp
    rcases hp with rfl | hp
    · rfl
    · exact preservedCommits_logical c vis (m :: rest) p (by simpa [preservedCommits] using hp)

theorem step_logical (r : Reader) (mint : String → Nat → String) (s : Fold) (c : Commit) :
    ∀ p ∈ (step r mint s c).commits, p ∈ s.commits ∨ (p.logicalId = c.id ∧ ¬ silent r c) := by
  unfold step
  split
  · next native hn =>
    intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact .inl h
    | inr h => exact .inr ⟨preservedCommits_logical c _ native p h, fun hs => by rw [hs.1] at hn; cases hn⟩
  · next hn =>
    split
    · exact fun p hp => .inl hp
    · next hv =>
      intro p hp
      cases List.mem_append.mp hp with
      | inl h => exact .inl h
      | inr h => simp at h; subst h; exact .inr ⟨rfl, fun hs => hv hs.2⟩

theorem setStep_logical (r : Viewer) (find : String → Option Commit) (st : State) (s : ChangeSet) :
    ∀ p ∈ (setStep r find st s).fold.commits,
      p ∈ st.fold.commits ∨ p.logicalId = ((s.sourceUpdate.bind find).map (·.id)).getD s.id := by
  show ∀ p ∈ (if setChanges r.view st (s.sourceUpdate.bind find) s = [] then st.fold else _).commits, _
  split
  · exact fun p hp => .inl hp
  · intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact .inl h
    | inr h => simp at h; subst h; exact .inr (by simp [Option.map_bind])

namespace Observer

variable {o : Observer}

theorem run_logical {refs : List String} (hnf : (fullView o.start == some o.view) = false) (base : Nat)
    {fa fb : String → Option Commit} (henv : ∀ x ∈ refs, o.SameLookup (fa x) (fb x))
    {P : String → Prop} {sa sb : List FoldStep}
    (hsteps : o.Steps refs sa sb) :
    (∀ c d v, FoldStep.commit c ∈ sa → FoldStep.commit d ∈ sb → o.SameCommit c d → v ∈ o.versions →
      Indistinct (o.reader v) c d → P c.id) →
    (∀ s t, FoldStep.set s ∈ sa → FoldStep.set t ∈ sb → o.SameSet refs s t →
      P (((s.sourceUpdate.bind fa).map (·.id)).getD s.id)) →
    ∀ {a b : State}, o.SameState a b → (∀ p ∈ a.fold.commits, P p.logicalId) →
      ∀ p ∈ (sa.foldl (foldStep ⟨o.view, base⟩ fa) a).fold.commits, P p.logicalId := by
  induction hsteps with
  | nil => intro _ _ _ _ _ hp; exact hp
  | commitLeft hc _ ih =>
    intro hcommit hset a b h hp
    have h' := commitStep_unseen hnf base h hc
    refine ih (fun c d v hc hd => hcommit c d v (List.mem_cons_of_mem _ hc) hd)
      (fun s t hs ht => hset s t (List.mem_cons_of_mem _ hs) ht) h' ?_
    intro p hpm
    exact hp p (by rw [← h'.fold.trans h.fold.symm]; exact hpm)
  | commitRight hc _ ih =>
    intro hcommit hset a b h hp
    exact ih (fun c d v hc hd => hcommit c d v hc (List.mem_cons_of_mem _ hd))
      (fun s t hs ht => hset s t hs (List.mem_cons_of_mem _ ht))
      (commitStep_unseen hnf base h.symm hc).symm hp
  | setLeft hs _ ih =>
    intro hcommit hset a b h hp
    have h' := setStep_unseen base fa h hs
    refine ih (fun c d v hc hd => hcommit c d v (List.mem_cons_of_mem _ hc) hd)
      (fun s t hs ht => hset s t (List.mem_cons_of_mem _ hs) ht) h' ?_
    intro p hpm
    exact hp p (by rw [← h'.fold.trans h.fold.symm]; exact hpm)
  | setRight hs _ ih =>
    intro hcommit hset a b h hp
    exact ih (fun c d v hc hd => hcommit c d v hc (List.mem_cons_of_mem _ hd))
      (fun s t hs ht => hset s t hs (List.mem_cons_of_mem _ ht))
      (setStep_unseen base fb h.symm hs).symm hp
  | @commit c d _ _ hcd _ ih =>
    intro hcommit hset a b h hp
    refine ih (fun c d v hc hd => hcommit c d v (List.mem_cons_of_mem _ hc) (List.mem_cons_of_mem _ hd))
      (fun s t hs ht => hset s t (List.mem_cons_of_mem _ hs) (List.mem_cons_of_mem _ ht))
      (commitStep_same hnf base h hcd) ?_
    intro p hpm
    cases step_logical _ _ _ c p hpm with
    | inl hold => exact hp p hold
    | inr hnew =>
      rw [hnew.1]
      cases hcd.observed a.views h.listed with
      | inl hi => exact hcommit c d a.views List.mem_cons_self List.mem_cons_self hcd h.listed hi
      | inr hs => exact absurd (reader_of_fold hnf a.views ▸ hs.1) hnew.2
  | @set s t _ _ hst _ ih =>
    intro hcommit hset a b h hp
    refine ih (fun c d v hc hd => hcommit c d v (List.mem_cons_of_mem _ hc) (List.mem_cons_of_mem _ hd))
      (fun s t hs ht => hset s t (List.mem_cons_of_mem _ hs) (List.mem_cons_of_mem _ ht))
      (setStep_same (fb := fb) base h hst ?_) ?_
    · rw [← hst.sourceUpdateIsNamed]
      cases hsu : s.sourceUpdate with
      | none => exact .none
      | some x =>
        exact henv x (hst.sourceUpdateIsListed x hsu)
    · intro p hpm
      cases setStep_logical _ fa a s p hpm with
      | inl hold => exact hp p hold
      | inr hnew => rw [hnew]; exact hset s t List.mem_cons_self List.mem_cons_self hst

end Observer
