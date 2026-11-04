import Projection.Steps

theorem filterMap_congr_mem {α β} {f g : α → Option β} :
    ∀ {l : List α}, (∀ x ∈ l, f x = g x) → l.filterMap f = l.filterMap g
  | [], _ => rfl
  | x :: rest, h => by
    rw [List.filterMap_cons, List.filterMap_cons, h x List.mem_cons_self,
      filterMap_congr_mem (fun y hy => h y (List.mem_cons_of_mem _ hy))]

theorem relabelEmit_hidden {before after : String → String → Bool} {source : Option Commit}
    {rl : Relabel} (hb : before rl.path rl.oldLabel = false) (ha : after rl.path rl.newLabel = false) :
    relabelEmit before after source rl = none := by
  unfold relabelEmit; rw [hb, ha]; simp

theorem treeEmit_hidden {before after : String → String → Bool} {relabels : List Relabel} {e : Entry}
    (hb : before e.path e.label = false) (ha : after e.path e.label = false) :
    treeEmit before after relabels e = none := by
  unfold treeEmit; rw [hb, ha]; simp

namespace Observer

variable {o : Observer}

theorem SameTree.symm {ta tb : Tree} (h : o.SameTree ta tb) : o.SameTree tb ta :=
  ⟨h.publicFilesAreDisclosed.symm, h.confinedRight, h.confinedLeft⟩

theorem SameTree.unseenCommit {ta tb : Tree} {c : Commit} (h : o.SameTree ta tb)
    (hc : o.UnseenCommit c) : o.SameTree (ta.applyCommit c) tb := by
  refine ⟨?_, o.confined_applyCommit hc.confined h.confinedLeft, h.confinedRight⟩
  rw [publicFiles_applyCommit, hc.noPublicChanges]
  exact h.publicFilesAreDisclosed

theorem SameTree.commit {ta tb : Tree} {c d : Commit} (h : o.SameTree ta tb)
    (hcd : o.SameCommit c d) : o.SameTree (ta.applyCommit c) (tb.applyCommit d) := by
  refine ⟨?_, o.confined_applyCommit hcd.confinedLeft h.confinedLeft,
    o.confined_applyCommit hcd.confinedRight h.confinedRight⟩
  rw [publicFiles_applyCommit, publicFiles_applyCommit, hcd.publicChangesAreDisclosed,
    h.publicFilesAreDisclosed]

theorem SameTree.unseenSet {ta tb : Tree} {s : ChangeSet} (h : o.SameTree ta tb)
    (hs : o.UnseenSet s) : o.SameTree (ta.relabel s) tb := by
  refine ⟨?_, o.confined_relabel hs.confined h.confinedLeft, h.confinedRight⟩
  rw [publicFiles_relabel, hs.noPublicRelabels]
  exact h.publicFilesAreDisclosed

theorem SameTree.set {refs : List String} {ta tb : Tree} {s t : ChangeSet} (h : o.SameTree ta tb)
    (hst : o.SameSet refs s t) : o.SameTree (ta.relabel s) (tb.relabel t) := by
  refine ⟨?_, o.confined_relabel hst.confinedLeft h.confinedLeft,
    o.confined_relabel hst.confinedRight h.confinedRight⟩
  rw [publicFiles_relabel, publicFiles_relabel, hst.publicRelabelsAreDisclosed,
    h.publicFilesAreDisclosed]

structure SameState (o : Observer) (a b : State) : Prop where
  fold : a.fold = b.fold
  views : a.views = b.views
  listed : a.views ∈ o.versions
  tree : o.SameTree a.tree b.tree

theorem SameState.symm {a b : State} (h : o.SameState a b) : o.SameState b a :=
  ⟨h.fold.symm, h.views.symm, h.views ▸ h.listed, h.tree.symm⟩

theorem reader_of_fold (hnf : (fullView o.start == some o.view) = false) (v : Views) :
    o.reader v = foldReader o.view v := by
  simp [reader, runReader, hnf]

theorem commitStep_unseen (hnf : (fullView o.start == some o.view) = false) (base : Nat)
    {a b : State} {c : Commit} (h : o.SameState a b) (hc : o.UnseenCommit c) :
    o.SameState (commitStep ⟨o.view, base⟩ a c) b := by
  have hs := hc.unshown a.views h.listed
  rw [reader_of_fold hnf] at hs
  exact ⟨(step_silent _ _ _ _ hs).trans h.fold, h.views, h.listed, h.tree.unseenCommit hc⟩

theorem commitStep_same (hnf : (fullView o.start == some o.view) = false) (base : Nat)
    {a b : State} {c d : Commit} (h : o.SameState a b) (hcd : o.SameCommit c d) :
    o.SameState (commitStep ⟨o.view, base⟩ a c) (commitStep ⟨o.view, base⟩ b d) := by
  refine ⟨?_, h.views, h.listed, h.tree.commit hcd⟩
  show step _ _ a.fold c = step _ _ b.fold d
  rw [h.fold, h.views]
  have hobs := hcd.observed b.views (h.views ▸ h.listed)
  rw [reader_of_fold hnf] at hobs
  cases hobs with
  | inl hi => exact step_indistinct _ _ _ _ _ hi
  | inr hs => rw [step_silent _ _ _ _ hs.1, step_silent _ _ _ _ hs.2]

theorem relabels_unseen {s : ChangeSet} (hs : o.publicRelabels s = []) :
    ∀ rl ∈ s.relabels, o.secret rl.path = true := by
  intro rl hrl
  have := List.filter_eq_nil_iff.mp hs rl hrl
  simpa [low] using this

theorem setStep_unseen (base : Nat) (find : String → Option Commit)
    {a b : State} {s : ChangeSet} (h : o.SameState a b) (hs : o.UnseenSet s) :
    o.SameState (setStep ⟨o.view, base⟩ find a s) b := by
  have htr : a.transition s = (a.views, a.views) := by unfold State.transition; rw [hs.noTransition]
  have hchanges : setChanges o.view a (s.sourceUpdate.bind find) s = [] := by
    unfold setChanges
    rw [htr, hs.noTransition]
    simp only [Option.isSome_none, Bool.false_eq_true, ite_false, List.append_nil]
    rw [List.filterMap_eq_nil_iff]
    intro rl hrl
    have hsec := relabels_unseen hs.noPublicRelabels rl hrl
    exact relabelEmit_hidden ((hs.confined rl hrl hsec).1 a.views h.listed)
      ((hs.confined rl hrl hsec).2 a.views h.listed)
  refine ⟨?_, ?_, ?_, h.tree.unseenSet hs⟩
  · show (if setChanges o.view a (s.sourceUpdate.bind find) s = [] then a.fold else _) = b.fold
    simp only [hchanges, ↓reduceIte]; exact h.fold
  · show (a.transition s).2 = b.views
    rw [htr]; exact h.views
  · show (a.transition s).2 ∈ o.versions
    rw [htr]; exact h.listed

theorem sourceShows_same {v : Views} (hv : v ∈ o.versions) {sa sb : Option Commit}
    (hsrc : o.SameLookup sa sb) (path : String) :
    sourceShows (v.shows o.view) sa path = sourceShows (v.shows o.view) sb path := by
  cases hsrc with
  | none => rfl
  | @some c d hcd =>
    show c.changes.any _ = d.changes.any _
    have hside : ∀ {e : Commit}, o.Confined e →
        e.changes.any (fun ch => ch.path == path && v.shows o.view ch.path ch.label) =
          (o.publicChanges e).any (fun ch => ch.path == path && v.shows o.view ch.path ch.label) := by
      intro e he
      apply any_filter_of
      intro ch hch hlow
      rw [(he ch hch (o.secret_of_not_low hlow)) v hv]
      simp
    rw [hside hcd.confinedLeft, hside hcd.confinedRight, hcd.publicChangesAreDisclosed]

theorem relabels_emit_same {before after : Views} (hb : before ∈ o.versions) (ha : after ∈ o.versions)
    {sa sb : Option Commit} (hsrc : o.SameLookup sa sb) {s t : ChangeSet} (hs : o.ConfinedSet s)
    (ht : o.ConfinedSet t) (hst : o.publicRelabels s = o.publicRelabels t) :
    s.relabels.filterMap (relabelEmit (before.shows o.view) (after.shows o.view) sa) =
      t.relabels.filterMap (relabelEmit (before.shows o.view) (after.shows o.view) sb) := by
  have hside : ∀ {u : ChangeSet} (src : Option Commit), o.ConfinedSet u →
      u.relabels.filterMap (relabelEmit (before.shows o.view) (after.shows o.view) src) =
        (o.publicRelabels u).filterMap (relabelEmit (before.shows o.view) (after.shows o.view) src) := by
    intro u src hu
    apply filterMap_filter_of
    intro rl hrl hlow
    have hh := hu rl hrl (o.secret_of_not_low hlow)
    exact relabelEmit_hidden (hh.1 before hb) (hh.2 after ha)
  rw [hside sa hs, hside sb ht, hst]
  have hemit : relabelEmit (before.shows o.view) (after.shows o.view) sa =
      relabelEmit (before.shows o.view) (after.shows o.view) sb := by
    funext rl
    unfold relabelEmit
    rw [sourceShows_same hb hsrc]
  rw [hemit]

theorem tree_emit_same {before after : Views} (hb : before ∈ o.versions) (ha : after ∈ o.versions)
    {ta tb : Tree} (htree : o.SameTree ta tb) {s t : ChangeSet}
    (hst : o.publicRelabels s = o.publicRelabels t) :
    ta.filterMap (treeEmit (before.shows o.view) (after.shows o.view) s.relabels) =
      tb.filterMap (treeEmit (before.shows o.view) (after.shows o.view) t.relabels) := by
  have hside : ∀ {u : Tree} (rls : List Relabel), o.ConfinedTree u →
      u.filterMap (treeEmit (before.shows o.view) (after.shows o.view) rls) =
        (o.publicFiles u).filterMap (treeEmit (before.shows o.view) (after.shows o.view) rls) := by
    intro u rls hu
    apply filterMap_filter_of
    intro e he hlow
    have hh := hu e he (o.secret_of_not_low hlow)
    exact treeEmit_hidden (hh before hb) (hh after ha)
  rw [hside s.relabels htree.confinedLeft, hside t.relabels htree.confinedRight,
    htree.publicFilesAreDisclosed]
  apply filterMap_congr_mem
  intro e he
  have helow : o.low e.path = true := (List.mem_filter.mp he).2
  have hany : ∀ (u : ChangeSet), u.relabels.any (·.path == e.path) =
      (o.publicRelabels u).any (·.path == e.path) := by
    intro u
    apply any_filter_of
    intro rl _ hlow
    cases heq : (rl.path == e.path) with
    | false => rfl
    | true => rw [beq_iff_eq.mp heq, helow] at hlow; cases hlow
  unfold treeEmit
  rw [hany s, hany t, hst]

theorem transition_listed {a : State} {s : ChangeSet} (hl : a.views ∈ o.versions)
    (hv : ∀ t, s.views = some t → t.before ∈ o.versions ∧ t.after ∈ o.versions) :
    (a.transition s).1 ∈ o.versions ∧ (a.transition s).2 ∈ o.versions := by
  unfold State.transition
  cases h : s.views with
  | none => exact ⟨hl, hl⟩
  | some t => exact hv t h

theorem setStep_same {refs : List String} (base : Nat) {fa fb : String → Option Commit}
    {a b : State} {s t : ChangeSet} (h : o.SameState a b) (hst : o.SameSet refs s t)
    (hsrc : o.SameLookup (s.sourceUpdate.bind fa) (t.sourceUpdate.bind fb)) :
    o.SameState (setStep ⟨o.view, base⟩ fa a s) (setStep ⟨o.view, base⟩ fb b t) := by
  have htr : a.transition s = b.transition t := by
    unfold State.transition; rw [hst.transitionIsDisclosed, h.views]
  have hlisted := transition_listed h.listed hst.versionsAreListed
  have hchanges : setChanges o.view a (s.sourceUpdate.bind fa) s =
      setChanges o.view b (t.sourceUpdate.bind fb) t := by
    unfold setChanges
    simp only
    rw [relabels_emit_same hlisted.1 hlisted.2 hsrc hst.confinedLeft hst.confinedRight
        hst.publicRelabelsAreDisclosed,
      tree_emit_same hlisted.1 hlisted.2 h.tree hst.publicRelabelsAreDisclosed, htr,
      hst.transitionIsDisclosed]
  have hid : ((s.sourceUpdate.bind fa).map (·.id)).getD s.id =
      ((t.sourceUpdate.bind fb).map (·.id)).getD t.id := by
    generalize s.sourceUpdate.bind fa = sa at hsrc ⊢
    generalize t.sourceUpdate.bind fb = sb at hsrc ⊢
    cases hsrc with
    | none => exact hst.setIdentityIsDisclosed
    | some hcd => simp [hcd.sourceIdentityIsDisclosed]
  have hsome : (s.sourceUpdate.bind fa).isSome = (t.sourceUpdate.bind fb).isSome := by
    generalize s.sourceUpdate.bind fa = sa at hsrc ⊢
    generalize t.sourceUpdate.bind fb = sb at hsrc ⊢
    cases hsrc <;> rfl
  have hmsg : ∀ changes, setMessage s (s.sourceUpdate.bind fa) changes =
      setMessage t (t.sourceUpdate.bind fb) changes := by
    intro changes
    unfold setMessage
    rw [hst.transitionIsDisclosed, hsome]
  refine ⟨?_, ?_, hlisted.2, h.tree.set hst⟩
  · show (if setChanges o.view a (s.sourceUpdate.bind fa) s = [] then a.fold else _) =
      (if setChanges o.view b (t.sourceUpdate.bind fb) t = [] then b.fold else _)
    rw [hchanges, h.fold, hid, hmsg, hst.setIdentityIsDisclosed]
  · show (a.transition s).2 = (b.transition t).2
    rw [htr]

theorem run_same {refs : List String} (hnf : (fullView o.start == some o.view) = false) (base : Nat)
    {fa fb : String → Option Commit} (henv : ∀ x ∈ refs, o.SameLookup (fa x) (fb x))
    {sa sb : List FoldStep} (hsteps : o.Steps refs sa sb) :
    ∀ {a b : State}, o.SameState a b →
      o.SameState (sa.foldl (foldStep ⟨o.view, base⟩ fa) a) (sb.foldl (foldStep ⟨o.view, base⟩ fb) b) := by
  induction hsteps with
  | nil => intro a b h; exact h
  | commitLeft hc _ ih => intro a b h; exact ih (commitStep_unseen hnf base h hc)
  | commitRight hc _ ih => intro a b h; exact ih (commitStep_unseen hnf base h.symm hc).symm
  | setLeft hs _ ih => intro a b h; exact ih (setStep_unseen base fa h hs)
  | setRight hs _ ih => intro a b h; exact ih (setStep_unseen base fb h.symm hs).symm
  | commit hcd _ ih => intro a b h; exact ih (commitStep_same hnf base h hcd)
  | @set s t _ _ hst _ ih =>
    intro a b h
    refine ih (setStep_same base h hst ?_)
    rw [← hst.sourceUpdateIsNamed]
    cases hsu : s.sourceUpdate with
    | none => exact .none
    | some x => exact henv x (hst.sourceUpdateIsListed x hsu)

end Observer
