import Projection.Model

structure Rule where
  path : String
  view : String
  deriving DecidableEq, Repr

structure Policy where
  default : String
  rules : List Rule
  deriving DecidableEq, Repr

def segments (path : String) : List String := (path.splitOn "/").filter (· ≠ "")

def joinPath (segs : List String) : String := "/" ++ "/".intercalate segs

def ruleAt (rules : List Rule) (path : String) : Option String :=
  (rules.find? (·.path == path)).map (·.view)

def nearestFirst (segs : List String) : List (List String) :=
  (List.range (segs.length + 1)).reverse.map segs.take

def nearest (rules : List Rule) (segs : List String) : Option String :=
  (nearestFirst segs).findSome? fun pre => ruleAt rules (joinPath pre)

def Policy.label (p : Policy) (full path : String) : String :=
  if path == "/.scope/RULES.md" then "public"
  else if isPrivateControlPath path then full
  else (nearest p.rules (segments path)).getD p.default

def keepLast (rules : List Rule) : List Rule :=
  rules.foldl (fun kept r => kept.filter (·.path != r.path) ++ [r]) []

def rulesByPath (rules : List Rule) : List Rule :=
  (keepLast rules).mergeSort fun a b => !decide (b.path < a.path)

def strictAncestors (path : String) : List String :=
  (List.range (segments path).length).map fun k => joinPath ((segments path).take k)

def island (rules : List Rule) : Option (String × String) :=
  rules.findSome? fun r =>
    if r.view == "public" then
      (strictAncestors r.path).findSome? fun a =>
        if ruleAt rules a == some "private" then some (r.path, a) else none
    else none

def Policy.addRules (p : Policy) (additions : List Rule) : Except (String × String) Policy :=
  if additions.isEmpty then .ok p
  else
    let merged := rulesByPath (p.rules ++ additions)
    match island merged with
    | some found => .error found
    | none => .ok { p with rules := merged }

theorem nearestFirst_succ (segs : List String) (n : Nat) :
    (List.range (n + 1)).reverse.map segs.take = segs.take n :: (List.range n).reverse.map segs.take := by
  rw [List.range_succ, List.reverse_append]
  rfl

theorem nearestFirst_snoc (segs : List String) (x : String) :
    nearestFirst (segs ++ [x]) = (segs ++ [x]) :: nearestFirst segs := by
  unfold nearestFirst
  rw [List.length_append, List.length_singleton, nearestFirst_succ,
    List.take_of_length_le (by simp)]
  congr 1
  refine List.map_congr_left fun k hk => ?_
  have : k ≤ segs.length := by simp at hk; omega
  exact List.take_append_of_le_length this

theorem nearestFirst_head (segs : List String) : ∃ rest, nearestFirst segs = segs :: rest := by
  unfold nearestFirst
  rw [nearestFirst_succ, List.take_of_length_le (Nat.le_refl _)]
  exact ⟨_, rfl⟩

theorem the_nearest_rule_wins (rules : List Rule) (segs : List String) (view : String)
    (h : ruleAt rules (joinPath segs) = some view) : nearest rules segs = some view := by
  obtain ⟨rest, hrest⟩ := nearestFirst_head segs
  simp [nearest, hrest, h]

theorem a_path_without_a_rule_inherits_its_parent (rules : List Rule) (segs : List String) (x : String)
    (h : ruleAt rules (joinPath (segs ++ [x])) = none) :
    nearest rules (segs ++ [x]) = nearest rules segs := by
  simp only [nearest, nearestFirst_snoc, List.findSome?_cons, h]

theorem the_root_has_only_its_own_rule (rules : List Rule) :
    nearest rules [] = ruleAt rules "/" := by
  have hj : joinPath [] = "/" := by simp [joinPath]
  have hf : nearestFirst [] = [[]] := rfl
  rw [nearest, hf, List.findSome?_cons, hj]
  cases ruleAt rules "/" <;> rfl

theorem only_segment_prefixes_apply (rules : List Rule) (segs : List String) (view : String)
    (h : nearest rules segs = some view) :
    ∃ pre, pre <+: segs ∧ ruleAt rules (joinPath pre) = some view := by
  unfold nearest at h
  obtain ⟨pre, hmem, hrule⟩ := List.exists_of_findSome?_eq_some h
  unfold nearestFirst at hmem
  obtain ⟨k, _, rfl⟩ := List.mem_map.mp hmem
  exact ⟨segs.take k, List.take_prefix k segs, hrule⟩

theorem strictAncestors_are_proper_prefixes (path a : String) :
    a ∈ strictAncestors path ↔
      ∃ pre, pre <+: segments path ∧ pre.length < (segments path).length ∧ a = joinPath pre := by
  unfold strictAncestors
  constructor
  · intro h
    obtain ⟨k, hk, rfl⟩ := List.mem_map.mp h
    have hk : k < (segments path).length := List.mem_range.mp hk
    exact ⟨(segments path).take k, List.take_prefix k _, by simp; omega, rfl⟩
  · rintro ⟨pre, hprefix, hlen, rfl⟩
    refine List.mem_map.mpr ⟨pre.length, List.mem_range.mpr hlen, ?_⟩
    rw [← List.prefix_iff_eq_take.mp hprefix]

def NoIsland (rules : List Rule) : Prop :=
  ∀ r ∈ rules, r.view = "public" → ∀ a ∈ strictAncestors r.path, ruleAt rules a ≠ some "private"

theorem island_none_iff (rules : List Rule) : island rules = none ↔ NoIsland rules := by
  unfold island NoIsland
  rw [List.findSome?_eq_none_iff]
  apply forall_congr'
  intro r
  apply forall_congr'
  intro _
  constructor
  · intro h hpub a ha hpriv
    have hp : (r.view == "public") = true := by simp [hpub]
    simp only [hp, ite_true] at h
    have := List.findSome?_eq_none_iff.mp h a ha
    simp [hpriv] at this
  · intro h
    split
    · next hpub =>
      apply List.findSome?_eq_none_iff.mpr
      intro a ha
      have := h (by simpa using hpub) a ha
      simp [this]
    · rfl

theorem the_public_island_rule_holds (p q : Policy) (additions : List Rule)
    (h : p.addRules additions = .ok q) :
    (additions = [] ∧ q = p) ∨ (q = { p with rules := rulesByPath (p.rules ++ additions) } ∧ NoIsland q.rules) := by
  unfold Policy.addRules at h
  split at h
  · next hempty => cases h; exact .inl ⟨List.isEmpty_iff.mp hempty, rfl⟩
  · simp only at h
    split at h
    · cases h
    · next hnone => cases h; exact .inr ⟨rfl, (island_none_iff _).mp hnone⟩

theorem a_public_island_is_rejected (p : Policy) (additions : List Rule) (child parent : String)
    (h : p.addRules additions = .error (child, parent)) :
    ∃ r ∈ rulesByPath (p.rules ++ additions), r.path = child ∧ r.view = "public" ∧
      parent ∈ strictAncestors child ∧ ruleAt (rulesByPath (p.rules ++ additions)) parent = some "private" := by
  unfold Policy.addRules at h
  split at h
  · cases h
  · simp only at h
    split at h
    · next found hfound =>
      cases h
      unfold island at hfound
      obtain ⟨r, hr, hsome⟩ := List.exists_of_findSome?_eq_some hfound
      split at hsome
      · next hpub =>
        obtain ⟨a, ha, hsa⟩ := List.exists_of_findSome?_eq_some hsome
        split at hsa
        · next hpriv =>
          cases hsa
          exact ⟨r, hr, rfl, by simpa using hpub, ha, by simpa using hpriv⟩
        · cases hsa
      · cases hsome
    · cases h

def Unique (rules : List Rule) : Prop := rules.Pairwise fun a b => a.path ≠ b.path

theorem find_unique {rules : List Rule} (hu : Unique rules) {r : Rule} (hr : r ∈ rules) :
    rules.find? (·.path == r.path) = some r := by
  induction rules with
  | nil => cases hr
  | cons s rest ih =>
    rw [List.find?_cons]
    have hu' := List.pairwise_cons.mp hu
    cases List.mem_cons.mp hr with
    | inl heq => subst heq; simp
    | inr hin =>
      have hne : (s.path == r.path) = false := by simpa using hu'.1 r hin
      simp [hne, ih hu'.2 hin]

theorem keepLast_spec :
    ∀ (rules kept : List Rule), Unique kept →
      Unique (rules.foldl (fun kept r => kept.filter (·.path != r.path) ++ [r]) kept) ∧
        ∀ path, ruleAt (rules.foldl (fun kept r => kept.filter (·.path != r.path) ++ [r]) kept) path =
          ((rules.reverse.find? (·.path == path)).map (·.view)).or (ruleAt kept path)
  | [], kept, hu => ⟨hu, fun _ => by simp⟩
  | r :: rest, kept, hu => by
    have hu' : Unique (kept.filter (·.path != r.path) ++ [r]) := by
      unfold Unique
      rw [List.pairwise_append]
      refine ⟨hu.filter _, List.pairwise_singleton _ _, ?_⟩
      intro a ha b hb
      simp at hb; subst hb
      simpa using (List.mem_filter.mp ha).2
    obtain ⟨hu'', heq⟩ := keepLast_spec rest _ hu'
    refine ⟨hu'', fun path => ?_⟩
    rw [List.foldl_cons, heq path, List.reverse_cons, List.find?_append]
    cases hrest : rest.reverse.find? (·.path == path) with
    | some _ => simp
    | none =>
      unfold ruleAt
      rw [List.find?_append]
      by_cases hp : r.path = path
      · subst hp
        have : (kept.filter (·.path != r.path)).find? (·.path == r.path) = none := by
          rw [List.find?_eq_none]
          intro x hx
          simpa using (List.mem_filter.mp hx).2
        simp [this]
      · have : (kept.filter (·.path != r.path)).find? (·.path == path) = kept.find? (·.path == path) := by
          rw [List.find?_filter]
          congr 1
          funext x
          by_cases hx : x.path = path
          · simp [hx, Ne.symm hp]
          · simp [hx]
        simp [this, hp]

theorem find_perm_unique {a b : List Rule} (hperm : a.Perm b) (hu : Unique a) (path : String) :
    a.find? (·.path == path) = b.find? (·.path == path) := by
  have hub : Unique b := (hperm.pairwise_iff (fun h => Ne.symm h)).mp hu
  cases ha : a.find? (·.path == path) with
  | some r =>
    have hr := List.mem_of_find?_eq_some ha
    have hp : r.path = path := by simpa using List.find?_some ha
    rw [← hp, find_unique hub (hperm.mem_iff.mp hr)]
  | none =>
    symm
    rw [List.find?_eq_none] at ha ⊢
    intro x hx
    exact ha x (hperm.mem_iff.mpr hx)

theorem the_last_rule_for_a_path_wins (rules : List Rule) (path : String) :
    ruleAt (rulesByPath rules) path = (rules.reverse.find? (·.path == path)).map (·.view) := by
  obtain ⟨hu, heq⟩ := keepLast_spec rules [] List.Pairwise.nil
  unfold rulesByPath keepLast
  have := heq path
  simp only [ruleAt, List.find?_nil, Option.map_none, Option.or_none] at this
  unfold ruleAt
  rw [← find_perm_unique (List.mergeSort_perm _ _).symm hu path]
  exact this
