import Projection.Noninterference

namespace Observer

variable {o : Observer}

theorem Histories.full {refs : List String} (hfull : (fullView o.start == some o.view) = true)
    (hl : o.start ∈ o.versions) {a b : List Commit} (h : o.Histories refs a b) :
    LowEq (fullReader o.view) a b := by
  have hr : o.reader o.start = fullReader o.view := by simp [reader, runReader, hfull]
  induction h with
  | nil => exact .nil
  | unseenLeft hc _ _ ih => exact .silentLeft (hr ▸ hc.unshown o.start hl) ih
  | unseenRight hc _ _ ih => exact .silentRight (hr ▸ hc.unshown o.start hl) ih
  | both hcd _ ih =>
    cases hcd.observed o.start hl with
    | inl hi => exact .both (hr ▸ hi) ih
    | inr hs => exact .silentLeft (hr ▸ hs.1) (.silentRight (hr ▸ hs.2) ih)

theorem tree_same {refs : List String} {sa sb : List FoldStep} (h : o.Steps refs sa sb) :
    ∀ {ta tb : Tree}, o.SameTree ta tb → o.SameTree (sa.foldl Tree.step ta) (sb.foldl Tree.step tb) := by
  induction h with
  | nil => intro _ _ h; exact h
  | commitLeft hc _ ih => intro _ _ ht; exact ih (ht.unseenCommit hc)
  | commitRight hc _ ih => intro _ _ ht; exact ih (ht.symm.unseenCommit hc).symm
  | setLeft hs _ ih => intro _ _ ht; exact ih (ht.unseenSet hs)
  | setRight hs _ ih => intro _ _ ht; exact ih (ht.symm.unseenSet hs).symm
  | commit hcd _ ih => intro _ _ ht; exact ih (ht.commit hcd)
  | set hst _ ih => intro _ _ ht; exact ih (ht.set hst)

theorem last_transition_same {refs : List String} {sa sb : List FoldStep} (h : o.Steps refs sa sb) :
    sa.reverse.findSome? FoldStep.transition = sb.reverse.findSome? FoldStep.transition := by
  induction h with
  | nil => rfl
  | commitLeft _ _ ih => simp [List.findSome?_append, FoldStep.transition, ih]
  | commitRight _ _ ih => simp [List.findSome?_append, FoldStep.transition, ih]
  | setLeft hs _ ih => simp [List.findSome?_append, FoldStep.transition, hs.noTransition, ih]
  | setRight hs _ ih => simp [List.findSome?_append, FoldStep.transition, hs.noTransition, ih]
  | commit _ _ ih => simp [List.findSome?_append, FoldStep.transition, ih]
  | set hst _ ih => simp [List.findSome?_append, FoldStep.transition, hst.transitionIsDisclosed, ih]

theorem last_transition_listed {refs : List String} {sa sb : List FoldStep} (h : o.Steps refs sa sb) :
    ∀ t, sa.reverse.findSome? FoldStep.transition = some t → t.after ∈ o.versions := by
  induction h with
  | nil => intro t ht; cases ht
  | commitLeft _ _ ih => intro t ht; simp [List.findSome?_append, FoldStep.transition] at ht; exact ih t ht
  | commitRight _ _ ih => exact ih
  | setLeft hs _ ih =>
    intro t ht; simp [List.findSome?_append, FoldStep.transition, hs.noTransition] at ht; exact ih t ht
  | setRight _ _ ih => exact ih
  | commit _ _ ih => intro t ht; simp [List.findSome?_append, FoldStep.transition] at ht; exact ih t ht
  | @set s _ _ _ hst _ ih =>
    intro t ht
    simp only [List.reverse_cons, List.findSome?_append] at ht
    cases hr : (List.findSome? FoldStep.transition _) with
    | some t' => rw [hr] at ht; cases ht; exact ih _ hr
    | none =>
      rw [hr] at ht
      simp [FoldStep.transition] at ht
      exact (hst.versionsAreListed t ht).2

theorem SetLists.initialViews {refs : List String} {sa sb : List ChangeSet} (h : o.SetLists refs sa sb)
    (current : Views) : _root_.initialViews sa current = _root_.initialViews sb current := by
  unfold _root_.initialViews
  congr 2
  induction h with
  | nil => rfl
  | unseenLeft hs _ ih => rw [List.findSome?_cons, hs.noTransition]; exact ih
  | unseenRight hs _ ih => rw [List.findSome?_cons (a := _), hs.noTransition]; exact ih
  | both hst _ ih => rw [List.findSome?_cons, List.findSome?_cons, hst.transitionIsDisclosed]; split <;> simp_all

end Observer

open Observer

theorem projection_noninterference (o : Observer) (refs : List String) (cursor : Cursor)
    (hstart : cursor.views = o.start) (hlisted : o.start ∈ o.versions)
    {ta tb : Tree} {ca cb : List Commit} {sa sb : List ChangeSet}
    (htree : o.SameTree ta tb) (hc : o.Histories refs ca cb) (hs : o.SetLists refs sa sb) :
    projectAfter cursor ta ca sa o.view = projectAfter cursor tb cb sb o.view := by
  unfold projectAfter
  rw [hstart]
  cases hfull : (fullView o.start == some o.view) with
  | true =>
    simp only [ite_true]
    rw [content_noninterference _ _ (hc.full hfull hlisted)]
  | false =>
    simp only [Bool.false_eq_true, ite_false]
    have hinit : o.SameState ⟨⟨[], cursor.last⟩, o.start, ta⟩ ⟨⟨[], cursor.last⟩, o.start, tb⟩ :=
      ⟨rfl, rfl, hlisted, htree⟩
    rw [(run_same hfull cursor.count (fun _ hx => hc.lookup hx) (foldSteps_steps hc hs) hinit).fold]

theorem graph_noninterference (o : Observer) (refs : List String) (current : Views)
    {ca cb : List Commit} {sa sb : List ChangeSet}
    (hstart : initialViews sa current = o.start) (hlisted : o.start ∈ o.versions)
    (hc : o.Histories refs ca cb) (hs : o.SetLists refs sa sb) :
    projectGraph ca sa current o.view = projectGraph cb sb current o.view := by
  unfold projectGraph
  rw [← hs.initialViews current]
  exact projection_noninterference o refs _ hstart hlisted
    ⟨rfl, fun _ h _ => (by cases h), fun _ h _ => (by cases h)⟩ hc hs

theorem foldSteps_without_sets (commits : List Commit) :
    foldSteps commits [] = commits.map .commit := by
  unfold foldSteps
  suffices h : ∀ seen, commitSteps [] commits seen = commits.map .commit by simp [h]
  induction commits with
  | nil => intro _; rfl
  | cons c rest ih => intro seen; simp [commitSteps, anchoredAfter, ih]

theorem commits_fold (r : Viewer) (find : String → Option Commit) :
    ∀ (commits : List Commit) (st : State),
      ((commits.map .commit).foldl (foldStep r find) st).fold =
        project (foldReader r.view st.views) r.mint st.fold commits
  | [], _ => rfl
  | c :: rest, st => commits_fold r find rest (commitStep r st c)

theorem history_noninterference (cursor : Cursor) (view : String) (ta tb : Tree)
    {ca cb : List Commit} (h : LowEq (runReader cursor.views view cursor.views) ca cb) :
    projectAfter cursor ta ca [] view = projectAfter cursor tb cb [] view := by
  unfold projectAfter
  cases hfull : (fullView cursor.views == some view) with
  | true =>
    simp only [ite_true]
    rw [content_noninterference _ _ (by simpa [runReader, hfull] using h)]
  | false =>
    simp only [Bool.false_eq_true, ite_false, foldSteps_without_sets]
    rw [commits_fold, commits_fold]
    exact congrArg Fold.commits
      (content_noninterference _ _ (by simpa [runReader, hfull] using h) _)

theorem incremental_noninterference (o : Observer) (refs : List String) (cursor : Cursor)
    (hstart : cursor.views = o.start) (hlisted : o.start ∈ o.versions)
    {ta tb : Tree} {pa pb qa qb : List Commit} {psa psb qsa qsb : List ChangeSet}
    (htree : o.SameTree ta tb) (hp : o.Histories refs pa pb) (hps : o.SetLists refs psa psb)
    (hq : { o with start := (resume cursor ta pa psa o.view).1.views }.Histories refs qa qb)
    (hqs : { o with start := (resume cursor ta pa psa o.view).1.views }.SetLists refs qsa qsb) :
    projectAfter (resume cursor ta pa psa o.view).1 (resume cursor ta pa psa o.view).2 qa qsa o.view =
      projectAfter (resume cursor tb pb psb o.view).1 (resume cursor tb pb psb o.view).2 qb qsb o.view := by
  have hsteps := foldSteps_steps hp hps
  have hprefix := projection_noninterference o refs cursor hstart hlisted htree hp hps
  have hcursor : (resume cursor ta pa psa o.view).1 = (resume cursor tb pb psb o.view).1 := by
    simp only [resume, Cursor.advance, hprefix, last_transition_same hsteps]
  have htrees : o.SameTree (resume cursor ta pa psa o.view).2 (resume cursor tb pb psb o.view).2 :=
    tree_same hsteps htree
  rw [← hcursor]
  refine projection_noninterference { o with start := (resume cursor ta pa psa o.view).1.views } refs _
    rfl ?_ ⟨htrees.1, htrees.2, htrees.3⟩ hq hqs
  show (((foldSteps pa psa).reverse.findSome? FoldStep.transition).map (·.after)).getD cursor.views ∈ o.versions
  cases ht : (foldSteps pa psa).reverse.findSome? FoldStep.transition with
  | none => exact hstart ▸ hlisted
  | some t => exact last_transition_listed hsteps t ht
