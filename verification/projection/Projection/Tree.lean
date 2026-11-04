import Projection.Observer

theorem filterMap_filter_of {α β} {f : α → Option β} {p : α → Bool} :
    ∀ {l : List α}, (∀ x ∈ l, p x = false → f x = none) → l.filterMap f = (l.filter p).filterMap f
  | [], _ => rfl
  | x :: rest, h => by
    have ih := filterMap_filter_of (l := rest) (fun y hy => h y (List.mem_cons_of_mem _ hy))
    cases hp : p x with
    | false => simp [hp, h x List.mem_cons_self hp, ih]
    | true => simp only [List.filter_cons, hp, ite_true, List.filterMap_cons]; rw [ih]

theorem any_filter_of {α} {q p : α → Bool} :
    ∀ {l : List α}, (∀ x ∈ l, p x = false → q x = false) → l.any q = (l.filter p).any q
  | [], _ => rfl
  | x :: rest, h => by
    have ih := any_filter_of (l := rest) (fun y hy => h y (List.mem_cons_of_mem _ hy))
    cases hp : p x with
    | false => simp [hp, h x List.mem_cons_self hp, ih]
    | true => simp [hp, ih]

namespace Observer

variable (o : Observer)

theorem filter_comm {α} (p q : α → Bool) (l : List α) :
    (l.filter p).filter q = (l.filter q).filter p := by
  rw [List.filter_filter, List.filter_filter]
  exact List.filter_congr (fun x _ => Bool.and_comm _ _)

theorem publicFiles_write (t : Tree) (ch : Change) :
    o.publicFiles (t.write ch) = if o.low ch.path then (o.publicFiles t).write ch else o.publicFiles t := by
  unfold publicFiles Tree.write
  cases hlow : o.low ch.path with
  | false =>
    have hkeep : ∀ e ∈ t.filter (o.low ·.path), (e.path != ch.path) = true := by
      intro e he
      simp only [List.mem_filter] at he
      simp only [bne_iff_ne, ne_eq]
      intro heq
      rw [heq, hlow] at he
      simp at he
    cases ch.content with
    | none =>
      simp only [ite_false, Bool.false_eq_true]
      rw [filter_comm, List.filter_eq_self.mpr hkeep]
    | some blob =>
      simp only [List.filter_cons, hlow, ite_false, Bool.false_eq_true]
      rw [filter_comm, List.filter_eq_self.mpr hkeep]
  | true =>
    cases ch.content with
    | none => simp only [ite_true]; rw [filter_comm]
    | some blob => simp only [List.filter_cons, hlow, ite_true]; rw [filter_comm]

theorem secret_of_not_low {p : String} (h : o.low p = false) : o.secret p = true := by
  simpa [low] using h

theorem publicFiles_writes :
    ∀ (cs : List Change) (t : Tree),
      o.publicFiles (cs.foldl Tree.write t) = (cs.filter (o.low ·.path)).foldl Tree.write (o.publicFiles t)
  | [], _ => rfl
  | ch :: rest, t => by
    rw [List.foldl_cons, publicFiles_writes rest, publicFiles_write]
    cases h : o.low ch.path <;> simp [h]

theorem publicFiles_applyCommit (t : Tree) (c : Commit) :
    o.publicFiles (t.applyCommit c) = (o.publicChanges c).foldl Tree.write (o.publicFiles t) :=
  publicFiles_writes o c.changes t

theorem confined_write {t : Tree} {ch : Change} (ht : o.ConfinedTree t)
    (hch : o.secret ch.path → o.Hidden ch.path ch.label) : o.ConfinedTree (t.write ch) := by
  unfold Tree.write
  have hrest : o.ConfinedTree (t.filter (·.path != ch.path)) :=
    fun e he => ht e (List.mem_filter.mp he).1
  cases ch.content with
  | none => exact hrest
  | some blob =>
    intro e he hs
    cases List.mem_cons.mp he with
    | inl heq => subst heq; exact hch hs
    | inr hin => exact hrest e hin hs

theorem confined_writes :
    ∀ (cs : List Change) (t : Tree), (∀ ch ∈ cs, o.secret ch.path → o.Hidden ch.path ch.label) →
      o.ConfinedTree t → o.ConfinedTree (cs.foldl Tree.write t)
  | [], _, _, ht => ht
  | ch :: rest, t, hcs, ht => by
    rw [List.foldl_cons]
    exact confined_writes rest _ (fun c hc => hcs c (List.mem_cons_of_mem _ hc))
      (o.confined_write ht (hcs ch List.mem_cons_self))

theorem confined_applyCommit {t : Tree} {c : Commit} (hc : o.Confined c) (ht : o.ConfinedTree t) :
    o.ConfinedTree (t.applyCommit c) :=
  o.confined_writes c.changes t hc ht

theorem publicFiles_relabelOne (t : Tree) (rl : Relabel) :
    o.publicFiles (t.relabelOne rl) =
      if o.low rl.path then (o.publicFiles t).relabelOne rl else o.publicFiles t := by
  unfold publicFiles Tree.relabelOne
  rw [List.filter_map]
  have hpath : ((fun e : Entry => o.low e.path) ∘
      fun e => if (e.path == rl.path) = true then { e with label := rl.newLabel } else e) =
      fun e => o.low e.path := by
    funext e; by_cases h : e.path = rl.path <;> simp [h]
  rw [hpath]
  cases hlow : o.low rl.path with
  | true => rfl
  | false =>
    simp only [Bool.false_eq_true, ite_false]
    refine (List.map_congr_left (g := id) ?_).trans (List.map_id _)
    intro e he
    have hne : (e.path == rl.path) = false := by
      have := (List.mem_filter.mp he).2
      cases heq : (e.path == rl.path) with
      | false => rfl
      | true => rw [beq_iff_eq.mp heq, hlow] at this; cases this
    simp [hne]

theorem publicFiles_relabels :
    ∀ (rls : List Relabel) (t : Tree),
      o.publicFiles (rls.foldl Tree.relabelOne t) =
        (rls.filter (o.low ·.path)).foldl Tree.relabelOne (o.publicFiles t)
  | [], _ => rfl
  | rl :: rest, t => by
    rw [List.foldl_cons, publicFiles_relabels rest, publicFiles_relabelOne]
    cases h : o.low rl.path <;> simp [h]

theorem publicFiles_relabel (t : Tree) (s : ChangeSet) :
    o.publicFiles (t.relabel s) = (o.publicRelabels s).foldl Tree.relabelOne (o.publicFiles t) :=
  publicFiles_relabels o s.relabels t

theorem confined_relabels :
    ∀ (rls : List Relabel) (t : Tree), (∀ rl ∈ rls, o.secret rl.path → o.Hidden rl.path rl.newLabel) →
      o.ConfinedTree t → o.ConfinedTree (rls.foldl Tree.relabelOne t)
  | [], _, _, ht => ht
  | rl :: rest, t, hrls, ht => by
    rw [List.foldl_cons]
    apply confined_relabels rest _ (fun r hr => hrls r (List.mem_cons_of_mem _ hr))
    intro e he hs
    unfold Tree.relabelOne at he
    obtain ⟨e', he', rfl⟩ := List.mem_map.mp he
    by_cases h : (e'.path == rl.path) = true
    · simp only [h, ite_true] at hs ⊢
      rw [beq_iff_eq.mp h] at hs ⊢
      exact hrls rl List.mem_cons_self hs
    · simp only [h, Bool.false_eq_true, ite_false] at hs ⊢
      exact ht e' he' hs

theorem confined_relabel {t : Tree} {s : ChangeSet} (hs : o.ConfinedSet s) (ht : o.ConfinedTree t) :
    o.ConfinedTree (t.relabel s) :=
  o.confined_relabels s.relabels t (fun rl hrl hsec => (hs rl hrl hsec).2) ht

end Observer
