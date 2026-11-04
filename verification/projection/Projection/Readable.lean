import Projection.Theorems

def runVersions (cursor : Cursor) (sets : List ChangeSet) : List Views :=
  cursor.views :: sets.flatMap fun s => match s.views with
    | some t => [t.before, t.after]
    | none => []

def Readable (versions : List Views) (view : String) (ch : Change) : Prop :=
  ∃ v ∈ versions, v.shows view ch.path ch.label = true

def AllReadable (versions : List Views) (view : String) (commits : List Projected) : Prop :=
  ∀ p ∈ commits, ∀ ch ∈ p.changes, Readable versions view ch

theorem preservedCommits_changes (c : Commit) (vis : List Change) :
    ∀ (native : List Native), ∀ p ∈ preservedCommits c vis native, ∀ ch ∈ p.changes, ch ∈ vis
  | [] => by intro p hp; cases hp
  | [n] => by intro p hp ch hch; simp [preservedCommits] at hp; subst hp; exact hch
  | n :: m :: rest => by
    intro p hp ch hch
    simp only [preservedCommits, List.mem_cons] at hp
    rcases hp with rfl | hp
    · cases hch
    · exact preservedCommits_changes c vis (m :: rest) p (by simpa [preservedCommits] using hp) ch hch

theorem step_readable (r : Reader) (mint : String → Nat → String) (s : Fold) (c : Commit)
    {P : Change → Prop} (hs : ∀ p ∈ s.commits, ∀ ch ∈ p.changes, P ch)
    (hvis : ∀ ch ∈ visible r c, P ch) :
    ∀ p ∈ (step r mint s c).commits, ∀ ch ∈ p.changes, P ch := by
  unfold step
  split
  · intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact hs p h
    | inr h => exact fun ch hch => hvis ch (preservedCommits_changes c _ _ p h ch hch)
  · split
    · exact hs
    · intro p hp
      cases List.mem_append.mp hp with
      | inl h => exact hs p h
      | inr h => simp at h; subst h; exact hvis

theorem visible_readable {versions : List Views} {view : String} {v : Views} (hv : v ∈ versions)
    (c : Commit) : ∀ ch ∈ visible (foldReader view v) c, Readable versions view ch := by
  intro ch hch
  exact ⟨v, hv, (List.mem_filter.mp hch).2⟩

theorem foldSteps_sets {commits : List Commit} {sets : List ChangeSet} {s : ChangeSet}
    (h : FoldStep.set s ∈ foldSteps commits sets) : s ∈ sets := by
  unfold foldSteps at h
  cases List.mem_append.mp h with
  | inl h => simp at h; exact h.1
  | inr h =>
    suffices ∀ (cs : List Commit) seen, FoldStep.set s ∈ commitSteps sets cs seen → s ∈ sets from
      this commits [] h
    intro cs
    induction cs with
    | nil => intro _ h; cases h
    | cons c rest ih =>
      intro seen h
      rw [commitSteps, List.mem_append, List.mem_cons] at h
      rcases h with (h | h) | h
      · cases h
      · obtain ⟨s', hs', heq⟩ := List.mem_map.mp h
        cases heq
        unfold anchoredAfter at hs'
        split at hs'
        · cases hs'
        · exact (List.mem_filter.mp hs').1
      · exact ih _ h

theorem transition_in_runVersions {cursor : Cursor} {sets : List ChangeSet} {s : ChangeSet}
    (hs : s ∈ sets) {t : Transition} (ht : s.views = some t) :
    t.before ∈ runVersions cursor sets ∧ t.after ∈ runVersions cursor sets := by
  unfold runVersions
  constructor <;> refine List.mem_cons_of_mem _ (List.mem_flatMap.mpr ⟨s, hs, ?_⟩) <;> simp [ht]

theorem relabelEmit_readable {before after : Views} {view : String} {source : Option Commit}
    {versions : List Views} (hb : before ∈ versions) (ha : after ∈ versions) (rl : Relabel) :
    ∀ ch ∈ relabelEmit (before.shows view) (after.shows view) source rl, Readable versions view ch := by
  intro ch hch
  unfold relabelEmit at hch
  split at hch
  · cases hch
  · split at hch
    · next hbf hat =>
      split at hch
      · cases hch
      · cases hc : rl.current with
        | none => rw [hc] at hch; cases hch
        | some blob => rw [hc] at hch; cases hch; exact ⟨after, ha, hat⟩
    · next hbt _ => cases hch; exact ⟨before, hb, hbt⟩
    · cases hch

theorem treeEmit_readable {before after : Views} {view : String} {relabels : List Relabel}
    {versions : List Views} (hb : before ∈ versions) (ha : after ∈ versions) (e : Entry) :
    ∀ ch ∈ treeEmit (before.shows view) (after.shows view) relabels e, Readable versions view ch := by
  intro ch hch
  unfold treeEmit at hch
  split at hch
  · cases hch
  · split at hch
    · next _ hat => cases hch; exact ⟨after, ha, hat⟩
    · next hbt _ => cases hch; exact ⟨before, hb, hbt⟩
    · cases hch

theorem setStep_readable {versions : List Views} (r : Viewer) (find : String → Option Commit)
    (st : State) (s : ChangeSet) (hst : AllReadable versions r.view st.fold.commits)
    (hb : (st.transition s).1 ∈ versions) (ha : (st.transition s).2 ∈ versions) :
    AllReadable versions r.view (setStep r find st s).fold.commits := by
  have hchanges : ∀ ch ∈ setChanges r.view st (s.sourceUpdate.bind find) s, Readable versions r.view ch := by
    intro ch hch
    unfold setChanges at hch
    cases List.mem_append.mp hch with
    | inl h =>
      obtain ⟨rl, _, hrl⟩ := List.mem_filterMap.mp h
      exact relabelEmit_readable hb ha rl ch hrl
    | inr h =>
      split at h
      · obtain ⟨e, _, he⟩ := List.mem_filterMap.mp ((List.mem_mergeSort).mp h)
        exact treeEmit_readable hb ha e ch he
      · cases h
  show AllReadable versions r.view
    (if setChanges r.view st (s.sourceUpdate.bind find) s = [] then st.fold else _).commits
  split
  · exact hst
  · intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact hst p h
    | inr h => simp at h; subst h; exact hchanges

theorem projected_changes_are_readable (cursor : Cursor) (tree : Tree) (commits : List Commit)
    (sets : List ChangeSet) (view : String) :
    AllReadable (runVersions cursor sets) view (projectAfter cursor tree commits sets view) := by
  unfold projectAfter
  split
  · next hfull =>
    have hshow : ∀ ch : Change, Readable (runVersions cursor sets) view ch :=
      fun ch => ⟨cursor.views, List.mem_cons_self, by simp [Views.shows, hfull]⟩
    suffices ∀ (cs : List Commit) (s : Fold), AllReadable (runVersions cursor sets) view s.commits →
        AllReadable (runVersions cursor sets) view
          (project (fullReader view) (Viewer.mint ⟨view, cursor.count⟩) s cs).commits from
      this commits _ (fun _ h => by cases h)
    intro cs
    induction cs with
    | nil => intro s hs; exact hs
    | cons c rest ih =>
      intro s hs
      exact ih _ (step_readable _ _ s c hs (fun ch _ => hshow ch))
  · suffices ∀ (steps : List FoldStep) (st : State), (∀ s, FoldStep.set s ∈ steps → s ∈ sets) →
        st.views ∈ runVersions cursor sets → AllReadable (runVersions cursor sets) view st.fold.commits →
        AllReadable (runVersions cursor sets) view
          (steps.foldl (foldStep ⟨view, cursor.count⟩ (lookup commits)) st).fold.commits from
      this _ _ (fun _ h => foldSteps_sets h) List.mem_cons_self (fun _ h => by cases h)
    intro steps
    induction steps with
    | nil => intro st _ _ h; exact h
    | cons step rest ih =>
      intro st hsets hviews hst
      have hrest : ∀ s, FoldStep.set s ∈ rest → s ∈ sets := fun s h => hsets s (List.mem_cons_of_mem _ h)
      cases step with
      | commit c =>
        exact ih _ hrest hviews (step_readable _ _ _ c hst (visible_readable hviews c))
      | set s =>
        have hmem := hsets s List.mem_cons_self
        have hlisted : (st.transition s).1 ∈ runVersions cursor sets ∧
            (st.transition s).2 ∈ runVersions cursor sets := by
          unfold State.transition
          cases ht : s.views with
          | none => exact ⟨hviews, hviews⟩
          | some t => exact transition_in_runVersions hmem ht
        exact ih _ hrest hlisted.2 (setStep_readable _ _ st s hst hlisted.1 hlisted.2)

theorem private_control_paths_reach_only_the_full_view (cursor : Cursor) (tree : Tree)
    (commits : List Commit) (sets : List ChangeSet) (view : String)
    (hpartial : ∀ v ∈ runVersions cursor sets, fullView v ≠ some view) :
    ∀ p ∈ projectAfter cursor tree commits sets view, ∀ ch ∈ p.changes, isPrivateControlPath ch.path = false := by
  intro p hp ch hch
  obtain ⟨v, hv, hshow⟩ := projected_changes_are_readable cursor tree commits sets view p hp ch hch
  have hnot : (fullView v == some view) = false := by simpa using hpartial v hv
  simp [Views.shows, hnot] at hshow
  exact hshow.2
