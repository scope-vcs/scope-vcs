import Projection.Model

inductive Reaches (views : Views) : String → String → Prop where
  | refl (a : String) : Reaches views a a
  | step {a b c : String} : Reaches views a b → c ∈ includesOf views b → Reaches views a c

theorem Reaches.trans {views : Views} {a b c : String} (hab : Reaches views a b)
    (hbc : Reaches views b c) : Reaches views a c := by
  induction hbc with
  | refl => exact hab
  | step _ h ih => exact .step ih h

theorem insertNews_mem :
    ∀ (xs seen : List String) (x : String), x ∈ xs.foldl insertNew seen ↔ x ∈ seen ∨ x ∈ xs
  | [], seen, x => by simp
  | y :: rest, seen, x => by
    rw [List.foldl_cons, insertNews_mem rest]
    unfold insertNew
    split
    · next h =>
      have hy : y ∈ seen := by simpa using h
      constructor
      · rintro (h | h)
        · exact .inl h
        · exact .inr (List.mem_cons_of_mem _ h)
      · rintro (h | h)
        · exact .inl h
        · cases List.mem_cons.mp h with
          | inl heq => exact .inl (heq ▸ hy)
          | inr h => exact .inr h
    · simp only [List.mem_append, List.mem_cons, List.not_mem_nil, or_false]
      constructor
      · rintro ((h | h) | h)
        · exact .inl h
        · exact .inr (.inl h)
        · exact .inr (.inr h)
      · rintro (h | h | h)
        · exact .inl (.inl h)
        · exact .inl (.inr h)
        · exact .inr h

theorem insertNews_nodup :
    ∀ (xs seen : List String), seen.Nodup → (xs.foldl insertNew seen).Nodup
  | [], _, h => h
  | y :: rest, seen, h => by
    rw [List.foldl_cons]
    apply insertNews_nodup rest
    unfold insertNew
    split
    · exact h
    · next hy =>
      have hy : y ∉ seen := by simpa using hy
      exact List.nodup_append.mpr ⟨h, (by simp : [y].Nodup), by
        intro a ha b hb; simp at hb; subst hb; intro heq; subst heq; exact hy ha⟩

theorem insertNews_prefix :
    ∀ (xs seen : List String), ∃ added, xs.foldl insertNew seen = seen ++ added
  | [], seen => ⟨[], by simp⟩
  | y :: rest, seen => by
    rw [List.foldl_cons]
    have hstep : ∃ e, insertNew seen y = seen ++ e := by
      unfold insertNew; split
      · exact ⟨[], by simp⟩
      · exact ⟨[y], rfl⟩
    obtain ⟨e, he⟩ := hstep
    obtain ⟨added, h⟩ := insertNews_prefix rest (insertNew seen y)
    exact ⟨e ++ added, by rw [h, he, List.append_assoc]⟩

theorem grow_mem (views : Views) (seen : List String) (x : String) :
    x ∈ grow views seen ↔ x ∈ seen ∨ ∃ a ∈ seen, x ∈ includesOf views a := by
  unfold grow
  rw [insertNews_mem]
  simp

theorem grow_closed {views : Views} {seen : List String}
    (h : (grow views seen).length = seen.length) : ∀ a ∈ seen, ∀ x ∈ includesOf views a, x ∈ seen := by
  obtain ⟨added, heq⟩ := insertNews_prefix (seen.flatMap (includesOf views)) seen
  have hgrow : grow views seen = seen ++ added := heq
  have hnil : added = [] := by
    rw [hgrow, List.length_append] at h
    exact List.eq_nil_of_length_eq_zero (by omega)
  intro a ha x hx
  have : x ∈ grow views seen := (grow_mem views seen x).mpr (.inr ⟨a, ha, hx⟩)
  simpa [hgrow, hnil] using this

theorem includesOf_mentioned (views : Views) (view a x : String) (hx : x ∈ includesOf views a) :
    x ∈ mentioned views view := by
  unfold includesOf at hx
  split at hx
  · next v hv =>
    have hmem := List.mem_of_find?_eq_some hv
    unfold ViewDef.direct at hx
    unfold mentioned
    apply List.mem_cons_of_mem
    split at hx
    · obtain ⟨w, hw, rfl⟩ := List.mem_map.mp hx
      exact List.mem_flatMap.mpr ⟨w, hw, List.mem_cons_self⟩
    · next included hinc =>
      exact List.mem_flatMap.mpr ⟨v, hmem, List.mem_cons_of_mem _ (by simp [hinc, hx])⟩
  · cases hx

theorem closeWithin_reaches (views : Views) :
    ∀ (n : Nat) (seen : List String) (x : String), x ∈ closeWithin views n seen →
      ∃ a ∈ seen, Reaches views a x
  | 0, seen, x, h => ⟨x, h, .refl x⟩
  | n + 1, seen, x, h => by
    unfold closeWithin at h
    split at h
    · exact ⟨x, h, .refl x⟩
    · obtain ⟨b, hb, hbx⟩ := closeWithin_reaches views n _ x h
      cases (grow_mem views seen b).mp hb with
      | inl hb => exact ⟨b, hb, hbx⟩
      | inr hb =>
        obtain ⟨a, ha, hab⟩ := hb
        exact ⟨a, ha, (Reaches.step (.refl a) hab).trans hbx⟩

theorem closeWithin_closed (views : Views) (pool : List String)
    (hpool : ∀ a x, x ∈ includesOf views a → x ∈ pool) :
    ∀ (n : Nat) (seen : List String), seen.Nodup → (∀ x ∈ seen, x ∈ pool) →
      pool.length < seen.length + n →
      (∀ x ∈ seen, x ∈ closeWithin views n seen) ∧
        ∀ a ∈ closeWithin views n seen, ∀ x ∈ includesOf views a, x ∈ closeWithin views n seen
  | 0, seen, hnodup, hsub, hlen => by
    have := List.Nodup.length_le_of_subset hnodup hsub
    omega
  | n + 1, seen, hnodup, hsub, hlen => by
    unfold closeWithin
    split
    · next h => exact ⟨fun _ hx => hx, grow_closed h⟩
    · next h =>
      obtain ⟨added, heq⟩ := insertNews_prefix (seen.flatMap (includesOf views)) seen
      have hgrow : grow views seen = seen ++ added := heq
      have hlonger : seen.length < (grow views seen).length := by
        rw [hgrow, List.length_append] at h ⊢
        cases added with
        | nil => simp at h
        | cons => simp
      have hrest := closeWithin_closed views pool hpool n (grow views seen)
        (insertNews_nodup _ _ hnodup)
        (by
          intro x hx
          cases (grow_mem views seen x).mp hx with
          | inl h => exact hsub x h
          | inr h => obtain ⟨a, _, hax⟩ := h; exact hpool a x hax)
        (by omega)
      refine ⟨fun x hx => hrest.1 x ((grow_mem views seen x).mpr (.inl hx)), hrest.2⟩

theorem labels_closed (views : Views) (view : String) :
    view ∈ labels views view ∧
      ∀ a ∈ labels views view, ∀ x ∈ includesOf views a, x ∈ labels views view := by
  have h := closeWithin_closed views (mentioned views view)
    (fun a x hx => includesOf_mentioned views view a x hx) (mentioned views view).length [view]
    (by simp : [view].Nodup) (by simp [mentioned]) (by simp)
  exact ⟨h.1 view List.mem_cons_self, h.2⟩

theorem labels_are_the_reflexive_transitive_closure_of_includes (views : Views) (view label : String) :
    label ∈ labels views view ↔ Reaches views view label := by
  constructor
  · intro h
    obtain ⟨a, ha, hreach⟩ := closeWithin_reaches views _ [view] label h
    simp at ha
    exact ha ▸ hreach
  · intro h
    induction h with
    | refl => exact (labels_closed views view).1
    | step _ hx ih => exact (labels_closed views view).2 _ ih _ hx

theorem read_access_is_transitive (views : Views) (reader target : String)
    (h : views.mayRead reader target = true) :
    ∀ label ∈ labels views target, label ∈ labels views reader := by
  intro label hl
  have hrt := (labels_are_the_reflexive_transitive_closure_of_includes views reader target).mp
    (by simpa [Views.mayRead] using h)
  exact (labels_are_the_reflexive_transitive_closure_of_includes views reader label).mpr
    (hrt.trans ((labels_are_the_reflexive_transitive_closure_of_includes views target label).mp hl))

structure Valid (views : Views) : Prop where
  unique : (views.map (·.id)).Nodup
  full : (views.filter (·.includes.isNone)).map (·.id) = ["private"]
  included : ∀ v ∈ views, ∀ included, v.includes = some included →
    "private" ∉ included ∧ ∀ x ∈ included, x ∈ views.map (·.id)

theorem valid_of (views : Views) (h : views.valid = true) : Valid views := by
  simp only [Views.valid, Bool.and_eq_true, decide_eq_true_eq, beq_iff_eq, List.all_eq_true] at h
  obtain ⟨⟨hnodup, hfull⟩, hpartial⟩ := h
  refine ⟨hnodup, hfull, ?_⟩
  intro v hv included hinc
  have := hpartial v hv
  rw [hinc] at this
  simp only [Bool.and_eq_true, Bool.not_eq_true', List.all_eq_true] at this
  refine ⟨by simpa using this.1, fun x hx => by simpa using this.2 x hx⟩

theorem fullView_filter : ∀ (views : Views),
    fullView views = ((views.filter (·.includes.isNone)).map (·.id)).head?
  | [] => rfl
  | v :: rest => by
    unfold fullView
    cases h : v.includes with
    | none => simp [h]
    | some _ =>
      simp [h]

theorem Valid.fullView {views : Views} (h : Valid views) : fullView views = some "private" := by
  rw [fullView_filter, h.full]; rfl

theorem Valid.find_full {views : Views} (h : Valid views) {v : ViewDef} (hv : v ∈ views)
    (hnone : v.includes = none) : v.id = "private" := by
  have : v.id ∈ (views.filter (·.includes.isNone)).map (·.id) :=
    List.mem_map.mpr ⟨v, List.mem_filter.mpr ⟨hv, by simp [hnone]⟩, rfl⟩
  rw [h.full] at this
  simpa using this

theorem Valid.not_includes_full {views : Views} (h : Valid views) (a : String) :
    "private" ∈ includesOf views a → a = "private" := by
  intro hx
  unfold includesOf at hx
  split at hx
  · next v hv =>
    have hmem := List.mem_of_find?_eq_some hv
    have hid : v.id = a := by simpa using List.find?_some hv
    unfold ViewDef.direct at hx
    split at hx
    · next hnone => exact hid ▸ h.find_full hmem hnone
    · next included hinc => exact absurd hx (h.included v hmem included hinc).1
  · cases hx

theorem only_the_full_view_reaches_the_full_label {views : Views} (hvalid : views.valid = true)
    (reader : String) :
    "private" ∈ labels views reader ↔ reader = "private" := by
  have h := valid_of views hvalid
  constructor
  · intro hl
    have hreach := (labels_are_the_reflexive_transitive_closure_of_includes views reader _).mp hl
    suffices ∀ l, Reaches views reader l → l = "private" → reader = "private" from this _ hreach rfl
    intro l hr
    induction hr with
    | refl => exact id
    | step _ hx ih => intro heq; subst heq; exact ih (h.not_includes_full _ hx)
  · intro heq
    subst heq
    exact (labels_closed views _).1

theorem read_access_is_monotone {views : Views} (hvalid : views.valid = true)
    (reader target : String) (hread : views.mayRead reader target = true) (path label : String)
    (hshows : views.shows target path label = true) : views.shows reader path label = true := by
  have hfull := (valid_of views hvalid).fullView
  unfold Views.shows at hshows ⊢
  rw [hfull] at hshows ⊢
  by_cases ht : target = "private"
  · subst ht
    have hl : "private" ∈ labels views reader := by simpa [Views.mayRead] using hread
    simp [(only_the_full_view_reaches_the_full_label hvalid reader).mp hl]
  · have hlabel : label ∈ labels views target ∧ isPrivateControlPath path = false := by
      simp only [Bool.or_eq_true, beq_iff_eq, Option.some.injEq, Bool.and_eq_true,
        List.contains_iff_mem, Bool.not_eq_true'] at hshows
      rcases hshows with h | h
      · exact absurd h.symm ht
      · exact h
    have := read_access_is_transitive views reader target hread label hlabel.1
    simp [this, hlabel.2]

theorem private_control_paths_are_shown_only_to_the_full_view (views : Views) (reader path label : String)
    (hcontrol : isPrivateControlPath path = true) (hshows : views.shows reader path label = true) :
    fullView views = some reader := by
  unfold Views.shows at hshows
  simp [hcontrol] at hshows
  exact hshows

theorem readable_views_are_the_readers_labels (views : Views) (reader target : String) :
    target ∈ views.readableBy (some reader) ↔
      target ∈ views.map (·.id) ∧ views.mayRead reader target = true := by
  simp [Views.readableBy, Views.mayRead]

theorem an_anonymous_reader_reads_only_the_anyone_view (views : Views) (target : String) :
    target ∈ views.readableBy none ↔ ∃ v, views.find? (·.anyone) = some v ∧ v.id = target := by
  simp [Views.readableBy]
