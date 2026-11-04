import Projection.PreviewTheorems

theorem mapM_congr {α β ε : Type} {f g : α → Except ε β} :
    ∀ {l : List α}, (∀ x ∈ l, f x = g x) → l.mapM f = l.mapM g
  | [], _ => rfl
  | x :: rest, h => by
    rw [List.mapM_cons, List.mapM_cons, h x List.mem_cons_self,
      mapM_congr (fun y hy => h y (List.mem_cons_of_mem _ hy))]

theorem writes_paths :
    ∀ (changes : List Change) (t : Tree) (e : Entry), e ∈ changes.foldl Tree.write t →
      e ∈ t ∨ ∃ ch ∈ changes, ch.path = e.path
  | [], _, _, h => .inl h
  | ch :: rest, t, e, h => by
    rw [List.foldl_cons] at h
    cases writes_paths rest _ e h with
    | inr h => obtain ⟨c, hc, hp⟩ := h; exact .inr ⟨c, List.mem_cons_of_mem _ hc, hp⟩
    | inl h =>
      unfold Tree.write at h
      split at h
      · cases List.mem_cons.mp h with
        | inl heq => subst heq; exact .inr ⟨ch, List.mem_cons_self, rfl⟩
        | inr h => exact .inl (List.mem_filter.mp h).1
      · exact .inl (List.mem_filter.mp h).1

theorem previewFiles_congr (oid : String → String) {la lb : String → String} {commits : List Projected}
    (h : ∀ p ∈ commits, ∀ ch ∈ p.changes, la ch.path = lb ch.path) :
    previewFiles oid la commits = previewFiles oid lb commits := by
  unfold previewFiles
  refine List.map_congr_left fun e he => ?_
  have he := List.mem_mergeSort.mp he
  unfold projectedTree at he
  cases writes_paths _ [] e he with
  | inl h => cases h
  | inr hch =>
    obtain ⟨ch, hch, hpath⟩ := hch
    obtain ⟨p, hp, hchp⟩ := List.mem_flatMap.mp hch
    rw [← hpath, h p hp ch hchp]

theorem classify_full {views : Views} {view : String} (h : fullView views = some view)
    (commits : List Commit) (x : String) : classify views view commits x = .allInView := by
  unfold classify
  cases lookup commits x <;> simp [the_full_view_sees_every_commit_in_view h]

namespace Observer

theorem visibility_same {o : Observer} {current : Views} (hnf : (fullView o.start == some o.view) = false)
    (hcurrent : current ∈ o.versions) {c d : Commit}
    (hcd : o.SameCommit c d) {v : Views} (hi : Indistinct (o.reader v) c d) :
    previewVisibility current o.view c = previewVisibility current o.view d := by
  rw [preview_visibility_is_relative_to_the_reader, preview_visibility_is_relative_to_the_reader]
  cases hcd.observed current hcurrent with
  | inl hc =>
    rw [reader_of_fold hnf] at hc
    rw [hc.visible, hc.hiddenChangePresenceIsDisclosed]
  | inr hs =>
    rw [reader_of_fold hnf] at hs
    have hempty := changes_nil_of_indistinct hi
    have hpartial : ∀ e : Commit, visible (foldReader o.view current) e = [] →
        partialCommit (foldReader o.view current) e = !e.changes.isEmpty := by
      intro e he
      unfold partialCommit
      rw [he]
      cases e.changes <;> simp
    rw [hpartial c hs.1.2, hpartial d hs.2.2, hs.1.2, hs.2.2]
    cases hc : c.changes with
    | nil => rw [hempty.mp hc]
    | cons x rest =>
      cases hd : d.changes with
      | nil => exact absurd (hempty.mpr hd) (by simp [hc])
      | cons => rfl

theorem preview_visibility_is_disclosed_only_by_source_updates (o : Observer) (refs : List String)
    (current : Views) {ca cb : List Commit} {sa sb : List ChangeSet}
    (hstart : initialViews sa current = o.start) (hlisted : o.start ∈ o.versions)
    (hcurrent : current ∈ o.versions) (hfull : fullView current = fullView o.start)
    (hc : o.Histories refs ca cb) (hs : o.SetLists refs sa sb) (hfa : FreshIds ca sa) (hfb : FreshIds cb sb)
    (sourceUpdateVisibilityIsDisclosed : ∀ s ∈ sa, ∀ x, s.sourceUpdate = some x → x ∈ refs →
      classify current o.view ca x = classify current o.view cb x) :
    ∀ p ∈ projectGraph ca sa current o.view,
      classify current o.view ca p.logicalId = classify current o.view cb p.logicalId := by
  unfold projectGraph projectAfter
  simp only [hstart]
  split
  · next hf =>
    have : fullView current = some o.view := by rw [hfull]; simpa using hf
    intro p _
    rw [classify_full this, classify_full this]
  · next hnf =>
    have hnf : (fullView o.start == some o.view) = false := by simpa using hnf
    refine run_logical (fa := lookup ca) (fb := lookup cb)
      (P := fun x => classify current o.view ca x = classify current o.view cb x)
      hnf 0 (fun _ hx => hc.lookup hx) (foldSteps_steps hc hs) ?_ ?_
      (a := ⟨⟨[], none⟩, o.start, []⟩) (b := ⟨⟨[], none⟩, o.start, []⟩)
      ⟨rfl, rfl, hlisted, ⟨rfl, fun _ h _ => (by cases h), fun _ h _ => (by cases h)⟩⟩ (fun _ h => by cases h)
    · intro c d v hcm hdm hcd _ hi
      have hc' := foldSteps_commits hcm
      have hd' := foldSteps_commits hdm
      unfold classify
      rw [lookup_unique hfa.1 hc', hcd.sourceIdentityIsDisclosed, lookup_unique hfb.1 hd']
      simp only [Option.map_some, Option.getD_some]
      exact visibility_same hnf hcurrent hcd hi
    · intro s t hsm htm hst
      have hs' := foldSteps_sets hsm
      have ht' := foldSteps_sets htm
      have hsetId : classify current o.view ca s.id = classify current o.view cb s.id := by
        unfold classify
        rw [lookup_missing (hfa.2 s hs'), hst.setIdentityIsDisclosed, lookup_missing (hfb.2 t ht')]
      cases hsu : s.sourceUpdate with
      | none => exact hsetId
      | some x =>
        simp only [Option.bind_some]
        cases hl : lookup ca x with
        | none => exact hsetId
        | some c =>
          simp only [Option.map_some, Option.getD_some]
          rw [lookup_found hl]
          exact sourceUpdateVisibilityIsDisclosed s hs' x hsu (hst.sourceUpdateIsListed x hsu)

end Observer

theorem preview_noninterference (o : Observer) (refs : List String) (oid : String → String)
    (current : Views) {ca cb : List Commit} {sa sb : List ChangeSet} {pa pb : Policy}
    {da db : String → Option NativeDetails}
    (hstart : initialViews sa current = o.start) (hlisted : o.start ∈ o.versions)
    (hcurrent : current ∈ o.versions) (hvalidStart : o.start.valid = true) (hvalid : current.valid = true)
    (hc : o.Histories refs ca cb) (hs : o.SetLists refs sa sb) (hfa : FreshIds ca sa) (hfb : FreshIds cb sb)
    (policyLabelsOfPublicPathsAreDisclosed :
      ∀ full path, o.low path = true → pa.label full path = pb.label full path)
    (nativeDetailsOfPreservedCommitsAreDisclosed : ∀ p ∈ projectGraph ca sa current o.view,
      ∀ native parents tree, p.materialization = .preserve native parents tree → da native = db native)
    (sourceUpdateVisibilityIsDisclosed : ∀ s ∈ sa, ∀ x, s.sourceUpdate = some x → x ∈ refs →
      classify current o.view ca x = classify current o.view cb x) :
    preview oid pa ca sa current o.view false da = preview oid pb cb sb current o.view false db ∧
      (preview oid pa ca sa current o.view true da).map withoutHiddenCounts =
        (preview oid pb cb sb current o.view true db).map withoutHiddenCounts := by
  have hfull := (valid_of _ hvalid).fullView
  have hfullStart := (valid_of _ hvalidStart).fullView
  have hproj : projectGraph cb sb current o.view = projectGraph ca sa current o.view :=
    (graph_noninterference o refs current hstart hlisted hc hs).symm
  have hpaths : ∀ p ∈ projectGraph ca sa current o.view, ∀ ch ∈ p.changes, o.low ch.path = true :=
    o.projected_paths_are_public refs ⟨0, none, initialViews sa current⟩ hstart hlisted
      (fun _ h _ => by cases h) hc hs
  have hfiles : previewFiles oid (pa.label "private") (projectGraph ca sa current o.view) =
      previewFiles oid (pb.label "private") (projectGraph ca sa current o.view) :=
    previewFiles_congr oid fun p hp ch hch =>
      policyLabelsOfPublicPathsAreDisclosed _ _ (hpaths p hp ch hch)
  have hclassify := o.preview_visibility_is_disclosed_only_by_source_updates refs current hstart hlisted
    hcurrent (by rw [hfull, hfullStart]) hc hs hfa hfb sourceUpdateVisibilityIsDisclosed
  have hcommits : (projectGraph ca sa current o.view).mapM (previewCommit (classify current o.view ca) da) =
      (projectGraph ca sa current o.view).mapM (previewCommit (classify current o.view cb) db) := by
    refine mapM_congr fun p hp => ?_
    unfold previewCommit
    rw [hclassify p hp]
    split
    · next native parents tree hm =>
      rw [nativeDetailsOfPreservedCommitsAreDisclosed p hp native parents tree hm]
    · rfl
  unfold preview
  rw [hfull, hproj]
  dsimp only
  rw [hfiles, hcommits]
  constructor
  · simp
  · cases (projectGraph ca sa current o.view).mapM (previewCommit (classify current o.view cb) db) with
    | error => rfl
    | ok cs => rfl

theorem hidden_counts_need_the_flag_and_a_partial_view (oid : String → String) (policy : Policy)
    (commits : List Commit) (sets : List ChangeSet) (current : Views) (view : String) (counts : Bool)
    (details : String → Option NativeDetails) (p : Preview)
    (h : preview oid policy commits sets current view counts details = .ok p)
    (hshown : counts = false ∨ fullView current = some view) :
    p.summary.hiddenFiles = 0 ∧ p.summary.hiddenCommits = 0 := by
  unfold preview at h
  split at h
  · cases h
  · next full hf =>
    have hnot : (view != full && counts) = false := by
      cases hshown with
      | inl hi => simp [hi]
      | inr hv => rw [hf] at hv; cases hv; simp
    simp only [hnot, Bool.false_eq_true, ite_false] at h
    cases hm : (projectGraph commits sets current view).mapM (previewCommit (classify current view commits) details) with
    | error => rw [hm] at h; cases h
    | ok cs => rw [hm] at h; cases h; exact ⟨rfl, rfl⟩

theorem preservedCommits_without_set (c : Commit) (vis : List Change) :
    ∀ (native : List Native), ∀ p ∈ preservedCommits c vis native, p.visibilityChangeSet = none
  | [] => by intro p hp; cases hp
  | [n] => by intro p hp; simp [preservedCommits] at hp; subst hp; rfl
  | n :: m :: rest => by
    intro p hp
    simp only [preservedCommits, List.mem_cons] at hp
    rcases hp with rfl | hp
    · rfl
    · exact preservedCommits_without_set c vis (m :: rest) p (by simpa [preservedCommits] using hp)

theorem step_without_set (r : Reader) (mint : String → Nat → String) (s : Fold) (c : Commit) :
    ∀ p ∈ (step r mint s c).commits, p ∈ s.commits ∨ p.visibilityChangeSet = none := by
  unfold step
  split
  · intro p hp
    cases List.mem_append.mp hp with
    | inl h => exact .inl h
    | inr h => exact .inr (preservedCommits_without_set c _ _ p h)
  · split
    · exact fun p hp => .inl hp
    · intro p hp
      cases List.mem_append.mp hp with
      | inl h => exact .inl h
      | inr h => simp at h; subst h; exact .inr rfl

def SetSourced (commits : List Commit) (sets : List ChangeSet) (p : Projected) : Prop :=
  ∀ id, p.visibilityChangeSet = some id →
    ∃ s ∈ sets, s.id = id ∧ p.logicalId = ((s.sourceUpdate.bind (lookup commits)).map (·.id)).getD s.id

theorem projected_sets_are_sourced (cursor : Cursor) (tree : Tree) (commits : List Commit)
    (sets : List ChangeSet) (view : String) :
    ∀ p ∈ projectAfter cursor tree commits sets view, SetSourced commits sets p := by
  unfold projectAfter
  split
  · suffices ∀ (cs : List Commit) (s : Fold), (∀ p ∈ s.commits, SetSourced commits sets p) →
        ∀ p ∈ (project (fullReader view) (Viewer.mint ⟨view, cursor.count⟩) s cs).commits,
          SetSourced commits sets p from this commits _ (fun _ h => by cases h)
    intro cs
    induction cs with
    | nil => intro s hs; exact hs
    | cons c rest ih =>
      intro s hs
      refine ih _ fun p hp => ?_
      cases step_without_set _ _ s c p hp with
      | inl h => exact hs p h
      | inr h => intro id hid; rw [h] at hid; cases hid
  · suffices ∀ (steps : List FoldStep) (st : State), (∀ s, FoldStep.set s ∈ steps → s ∈ sets) →
        (∀ p ∈ st.fold.commits, SetSourced commits sets p) →
        ∀ p ∈ (steps.foldl (foldStep ⟨view, cursor.count⟩ (lookup commits)) st).fold.commits,
          SetSourced commits sets p from
      this _ _ (fun _ h => foldSteps_sets h) (fun _ h => by cases h)
    intro steps
    induction steps with
    | nil => intro st _ h; exact h
    | cons step rest ih =>
      intro st hsets hst
      have hrest : ∀ s, FoldStep.set s ∈ rest → s ∈ sets := fun s h => hsets s (List.mem_cons_of_mem _ h)
      cases step with
      | commit c =>
        refine ih _ hrest fun p hp => ?_
        cases step_without_set _ _ st.fold c p hp with
        | inl h => exact hst p h
        | inr h => intro id hid; rw [h] at hid; cases hid
      | set s =>
        refine ih _ hrest fun p hp => ?_
        show SetSourced commits sets p
        have hp' : p ∈ (if setChanges view st (s.sourceUpdate.bind (lookup commits)) s = [] then st.fold
            else _).commits := hp
        split at hp'
        · exact hst p hp'
        · cases List.mem_append.mp hp' with
          | inl h => exact hst p h
          | inr h =>
            simp at h; subst h
            intro id hid
            cases hid
            exact ⟨s, hsets s List.mem_cons_self, rfl, by simp [Option.map_bind]⟩

theorem a_source_update_carries_its_sources_preview_visibility (current : Views) (view : String)
    (commits : List Commit) (sets : List ChangeSet) (details : String → Option NativeDetails)
    {p : Projected} {pc : PreviewCommit} {id : String} (hp : p ∈ projectGraph commits sets current view)
    (hset : p.visibilityChangeSet = some id)
    (hpc : previewCommit (classify current view commits) details p = .ok pc) :
    ∃ s ∈ sets, s.id = id ∧ ∀ c, s.sourceUpdate.bind (lookup commits) = some c →
      pc.visibility = previewVisibility current view c := by
  obtain ⟨s, hs, hid, hlogical⟩ := projected_sets_are_sourced _ _ _ _ _ p hp id hset
  refine ⟨s, hs, hid, fun c hc => ?_⟩
  have hvis : pc.visibility = classify current view commits p.logicalId := by
    unfold previewCommit at hpc
    split at hpc
    · split at hpc
      · cases hpc; rfl
      · cases hpc
    · cases hpc; rfl
  rw [hvis, hlogical, hc]
  obtain ⟨x, hx, hl⟩ := Option.bind_eq_some_iff.mp hc
  simp only [Option.map_some, Option.getD_some, classify]
  rw [lookup_found hl, hl]
  rfl
