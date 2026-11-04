import Projection.Readable

namespace Observer

variable {o : Observer}

theorem Histories.confined {refs : List String} {a b : List Commit} (h : o.Histories refs a b) :
    ∀ c ∈ a, o.Confined c := by
  induction h with
  | nil => intro _ h; cases h
  | unseenLeft hc _ _ ih =>
    intro c h
    cases List.mem_cons.mp h with
    | inl heq => exact heq ▸ hc.confined
    | inr h => exact ih c h
  | unseenRight _ _ _ ih => exact ih
  | both hcd _ ih =>
    intro c h
    cases List.mem_cons.mp h with
    | inl heq => exact heq ▸ hcd.confinedLeft
    | inr h => exact ih c h

def ListedSet (o : Observer) (s : ChangeSet) : Prop :=
  o.ConfinedSet s ∧ ∀ t, s.views = some t → t.before ∈ o.versions ∧ t.after ∈ o.versions

theorem SetLists.listed {refs : List String} {a b : List ChangeSet} (h : o.SetLists refs a b) :
    ∀ s ∈ a, o.ListedSet s := by
  induction h with
  | nil => intro _ h; cases h
  | unseenLeft hs _ ih =>
    intro s h
    cases List.mem_cons.mp h with
    | inl heq => subst heq; exact ⟨hs.confined, fun t ht => by rw [hs.noTransition] at ht; cases ht⟩
    | inr h => exact ih s h
  | unseenRight _ _ ih => exact ih
  | both hst _ ih =>
    intro s h
    cases List.mem_cons.mp h with
    | inl heq => subst heq; exact ⟨hst.confinedLeft, hst.versionsAreListed⟩
    | inr h => exact ih s h

end Observer

theorem foldSteps_commits {commits : List Commit} {sets : List ChangeSet} {c : Commit}
    (h : FoldStep.commit c ∈ foldSteps commits sets) : c ∈ commits := by
  unfold foldSteps at h
  cases List.mem_append.mp h with
  | inl h => simp at h
  | inr h =>
    suffices ∀ (cs : List Commit) seen, FoldStep.commit c ∈ commitSteps sets cs seen → c ∈ cs from
      this commits [] h
    intro cs
    induction cs with
    | nil => intro _ h; cases h
    | cons d rest ih =>
      intro seen h
      rw [commitSteps, List.mem_append, List.mem_cons] at h
      rcases h with (h | h) | h
      · cases h; exact List.mem_cons_self
      · simp at h
      · exact List.mem_cons_of_mem _ (ih _ h)

theorem relabelEmit_source {before after : String → String → Bool} {source : Option Commit}
    {rl : Relabel} {ch : Change} (h : relabelEmit before after source rl = some ch) :
    ch.path = rl.path ∧ (before rl.path rl.oldLabel = true ∨ after rl.path rl.newLabel = true) := by
  unfold relabelEmit at h
  split at h
  · cases h
  · split at h
    · next hat =>
      split at h
      · cases h
      · cases hc : rl.current with
        | none => rw [hc] at h; cases h
        | some blob => rw [hc] at h; cases h; exact ⟨rfl, .inr hat⟩
    · next hbt _ => cases h; exact ⟨rfl, .inl hbt⟩
    · cases h

theorem treeEmit_source {before after : String → String → Bool} {relabels : List Relabel} {e : Entry}
    {ch : Change} (h : treeEmit before after relabels e = some ch) :
    ch.path = e.path ∧ (before e.path e.label = true ∨ after e.path e.label = true) := by
  unfold treeEmit at h
  split at h
  · cases h
  · split at h
    · next hat => cases h; exact ⟨rfl, .inr hat⟩
    · next hbt _ => cases h; exact ⟨rfl, .inl hbt⟩
    · cases h

namespace Observer

variable {o : Observer}

theorem low_of_shown {v : Views} (hv : v ∈ o.versions) {path label : String}
    (hconfined : o.secret path = true → o.Hidden path label) (hshown : v.shows o.view path label = true) :
    o.low path = true := by
  cases hs : o.secret path with
  | false => simp [low, hs]
  | true => rw [hconfined hs v hv] at hshown; cases hshown

theorem setStep_public (r : Viewer) (hview : r.view = o.view) (find : String → Option Commit)
    (st : State) (s : ChangeSet) (hs : o.ListedSet s) (hviews : st.views ∈ o.versions)
    (htree : o.ConfinedTree st.tree) (hst : ∀ p ∈ st.fold.commits, ∀ ch ∈ p.changes, o.low ch.path = true) :
    ∀ p ∈ (setStep r find st s).fold.commits, ∀ ch ∈ p.changes, o.low ch.path = true := by
  have hlisted := transition_listed hviews hs.2
  have hchanges : ∀ ch ∈ setChanges r.view st (s.sourceUpdate.bind find) s, o.low ch.path = true := by
    intro ch hch
    rw [hview] at hch
    unfold setChanges at hch
    cases List.mem_append.mp hch with
    | inl h =>
      obtain ⟨rl, hrl, hemit⟩ := List.mem_filterMap.mp h
      obtain ⟨hpath, hshown⟩ := relabelEmit_source hemit
      rw [hpath]
      cases hshown with
      | inl hb => exact low_of_shown hlisted.1 (fun hsec => (hs.1 rl hrl hsec).1) hb
      | inr ha => exact low_of_shown hlisted.2 (fun hsec => (hs.1 rl hrl hsec).2) ha
    | inr h =>
      split at h
      · obtain ⟨e, he, hemit⟩ := List.mem_filterMap.mp ((List.mem_mergeSort).mp h)
        obtain ⟨hpath, hshown⟩ := treeEmit_source hemit
        rw [hpath]
        cases hshown with
        | inl hb => exact low_of_shown hlisted.1 (htree e he) hb
        | inr ha => exact low_of_shown hlisted.2 (htree e he) ha
      · cases h
  show ∀ p ∈ (if setChanges r.view st (s.sourceUpdate.bind find) s = [] then st.fold else _).commits,
    ∀ ch ∈ p.changes, o.low ch.path = true
  split
  · exact hst
  · intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact hst p h
    | inr h => simp at h; subst h; exact hchanges

theorem projected_paths_are_public (refs : List String) (cursor : Cursor)
    (hstart : cursor.views = o.start) (hlisted : o.start ∈ o.versions) {tree : Tree} {ca cb : List Commit}
    {sa sb : List ChangeSet} (htree : o.ConfinedTree tree) (hc : o.Histories refs ca cb)
    (hs : o.SetLists refs sa sb) :
    ∀ p ∈ projectAfter cursor tree ca sa o.view, ∀ ch ∈ p.changes, o.low ch.path = true := by
  have hcon := hc.confined
  have hsets := hs.listed
  unfold projectAfter
  split
  · next hfull =>
    rw [hstart] at hfull
    have hall : ∀ c ∈ ca, ∀ ch ∈ c.changes, o.low ch.path = true := by
      intro c hcm ch hch
      refine low_of_shown hlisted (hcon c hcm ch hch) ?_
      simp [Views.shows, hfull]
    suffices ∀ (cs : List Commit) (s : Fold), (∀ c ∈ cs, c ∈ ca) →
        (∀ p ∈ s.commits, ∀ ch ∈ p.changes, o.low ch.path = true) →
        ∀ p ∈ (project (fullReader o.view) (Viewer.mint ⟨o.view, cursor.count⟩) s cs).commits,
          ∀ ch ∈ p.changes, o.low ch.path = true from
      this ca _ (fun _ h => h) (fun _ h => by cases h)
    intro cs
    induction cs with
    | nil => intro s _ hs; exact hs
    | cons c rest ih =>
      intro s hmem hs
      exact ih _ (fun d hd => hmem d (List.mem_cons_of_mem _ hd))
        (step_readable _ _ s c hs (fun ch hch =>
          hall c (hmem c List.mem_cons_self) ch (List.mem_filter.mp hch).1))
  · suffices ∀ (steps : List FoldStep) (st : State),
        (∀ c, FoldStep.commit c ∈ steps → c ∈ ca) → (∀ s, FoldStep.set s ∈ steps → s ∈ sa) →
        st.views ∈ o.versions → o.ConfinedTree st.tree →
        (∀ p ∈ st.fold.commits, ∀ ch ∈ p.changes, o.low ch.path = true) →
        ∀ p ∈ (steps.foldl (foldStep ⟨o.view, cursor.count⟩ (lookup ca)) st).fold.commits,
          ∀ ch ∈ p.changes, o.low ch.path = true from
      this _ _ (fun _ h => foldSteps_commits h) (fun _ h => foldSteps_sets h) (hstart ▸ hlisted) htree
        (fun _ h => by cases h)
    intro steps
    induction steps with
    | nil => intro st _ _ _ _ h; exact h
    | cons step rest ih =>
      intro st hcommits hsetsIn hviews htr hst
      have hrc : ∀ c, FoldStep.commit c ∈ rest → c ∈ ca :=
        fun c h => hcommits c (List.mem_cons_of_mem _ h)
      have hrs : ∀ s, FoldStep.set s ∈ rest → s ∈ sa := fun s h => hsetsIn s (List.mem_cons_of_mem _ h)
      cases step with
      | commit c =>
        have hcm := hcommits c List.mem_cons_self
        refine ih _ hrc hrs hviews (o.confined_applyCommit (hcon c hcm) htr) ?_
        exact step_readable _ _ _ c hst (fun ch hch =>
          low_of_shown hviews (hcon c hcm ch (List.mem_filter.mp hch).1) (List.mem_filter.mp hch).2)
      | set s =>
        have hl := hsets s (hsetsIn s List.mem_cons_self)
        exact ih _ hrc hrs (transition_listed hviews hl.2).2 (o.confined_relabel hl.1 htr)
          (setStep_public _ rfl _ st s hl hviews htr hst)

end Observer
