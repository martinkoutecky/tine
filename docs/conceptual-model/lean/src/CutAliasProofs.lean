import CutSafetyProofs
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

/-- Proof notation for the existing fold, not a second alias algorithm. -/
private def aliasRun (acc xs : List String) : List String :=
  xs.foldl (fun kept n => if kept.any (fun k => Date13.target P D [] k == Date13.target P D [] n)
    then kept else kept ++ [n]) acc

private theorem alias_duplicate (acc : List String) (n : String) :
    acc.any (fun k => Date13.target P D [] k == Date13.target P D [] n) = true ↔
      Date13.target P D [] n ∈ acc.map (Date13.target P D []) := by
  simp [List.any_eq_true, List.mem_map]

private theorem alias_run_append (acc xs ys : List String) :
    aliasRun P D acc (xs ++ ys) = aliasRun P D (aliasRun P D acc xs) ys := by
  simp only [aliasRun, List.foldl_append]

private theorem alias_run_sublist (acc xs : List String) :
    (aliasRun P D acc xs).Sublist (acc ++ xs) := by
  induction xs generalizing acc with
  | nil => simp [aliasRun]
  | cons a xs ih =>
    simp only [aliasRun, List.foldl_cons]
    split
    · exact (ih acc).trans ((List.Sublist.cons a (List.Sublist.refl xs)).append_left acc)
    · simpa [aliasRun, List.append_assoc] using ih (acc ++ [a])

private theorem alias_run_prefix (acc xs : List String) :
    ∃ tail, aliasRun P D acc xs = acc ++ tail := by
  induction xs generalizing acc with
  | nil => exact ⟨[], by simp [aliasRun]⟩
  | cons a xs ih =>
    simp only [aliasRun, List.foldl_cons]
    split
    · exact ih acc
    · obtain ⟨tail,h⟩ := ih (acc ++ [a])
      exact ⟨a :: tail, by simpa [aliasRun, List.append_assoc] using h⟩

private theorem alias_run_coverage (acc xs : List String) (k : Page K) :
    k ∈ (aliasRun P D acc xs).map (Date13.target P D []) ↔
      k ∈ acc.map (Date13.target P D []) ∨ k ∈ xs.map (Date13.target P D []) := by
  induction xs generalizing acc with
  | nil => simp [aliasRun]
  | cons a xs ih =>
    change k ∈ (aliasRun P D
      (if acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a) then acc else acc ++ [a]) xs).map _ ↔ _
    by_cases hd : acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a) = true
    · rw [if_pos hd, ih]
      have ha := (alias_duplicate P D acc a).1 hd
      simp only [List.map_cons, List.mem_cons]
      constructor
      · intro h; rcases h with h | h
        · exact Or.inl h
        · exact Or.inr (Or.inr h)
      · intro h; rcases h with h | h | h
        · exact Or.inl h
        · subst k; exact Or.inl ha
        · exact Or.inr h
    · rw [if_neg hd, ih]
      simp only [List.map_append, List.map_cons, List.map_nil, List.mem_append,
        List.mem_cons, List.not_mem_nil, or_false]
      constructor
      · intro h; rcases h with (h | h) | h
        · exact Or.inl h
        · exact Or.inr (Or.inl h)
        · exact Or.inr (Or.inr h)
      · intro h; rcases h with h | h | h
        · exact Or.inl (Or.inl h)
        · exact Or.inl (Or.inr h)
        · exact Or.inr h

private theorem alias_run_nodup (acc xs : List String)
    (hn : (acc.map (Date13.target P D [])).Nodup) :
    ((aliasRun P D acc xs).map (Date13.target P D [])).Nodup := by
  induction xs generalizing acc with
  | nil => simpa [aliasRun] using hn
  | cons a xs ih =>
    change ((aliasRun P D
      (if acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a) then acc else acc ++ [a]) xs).map _).Nodup
    by_cases hd : acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a) = true
    · rw [if_pos hd]; exact ih acc hn
    · rw [if_neg hd]
      apply ih
      have ha : Date13.target P D [] a ∉ acc.map (Date13.target P D []) :=
        fun h => hd ((alias_duplicate P D acc a).2 h)
      rw [List.map_append]
      apply List.nodup_append.mpr
      refine ⟨hn, by simp, ?_⟩
      intro k hk j hj
      simp only [List.map_cons, List.map_nil, List.mem_cons, List.not_mem_nil, or_false] at hj
      subst j
      exact fun he => ha (he ▸ hk)

/-- Stable output order, including repeated equal names in the input. -/
theorem alias_union_order (xs : List String) : (uniqueNames P D xs).Sublist xs := by
  simpa [uniqueNames, aliasRun] using alias_run_sublist P D [] xs

/-- Every input identity has an output representative and none is invented. -/
theorem alias_union_coverage (xs : List String) (k : Page K) :
    k ∈ (uniqueNames P D xs).map (Date13.target P D []) ↔
      k ∈ xs.map (Date13.target P D []) := by
  simpa [uniqueNames, aliasRun] using alias_run_coverage P D [] xs k

theorem alias_union_nodup (xs : List String) :
    ((uniqueNames P D xs).map (Date13.target P D [])).Nodup := by
  simpa [uniqueNames, aliasRun] using alias_run_nodup P D [] xs (by simp)

/-- The exact multiplicity is one for present identities and zero otherwise. -/
theorem alias_union_count (xs : List String) (k : Page K) :
    ((uniqueNames P D xs).map (Date13.target P D [])).count k =
      if k ∈ xs.map (Date13.target P D []) then 1 else 0 := by
  by_cases h : k ∈ xs.map (Date13.target P D [])
  · rw [if_pos h]
    have hm := (alias_union_coverage P D xs k).2 h
    have hp := List.count_pos_iff.mpr hm
    have hl := List.nodup_iff_count.mp (alias_union_nodup P D xs) k
    omega
  · rw [if_neg h]
    apply List.count_eq_zero_of_not_mem
    exact fun hm => h ((alias_union_coverage P D xs k).1 hm)

/-- The actual first spelling survives. This uses an arbitrary prefix/suffix
decomposition and needs only that the prefix contains no earlier equal identity. -/
theorem alias_union_first (pre post : List String) (a : String)
    (hfirst : Date13.target P D [] a ∉ pre.map (Date13.target P D [])) :
    a ∈ uniqueNames P D (pre ++ a :: post) := by
  let acc := aliasRun P D [] pre
  have hn : Date13.target P D [] a ∉ acc.map (Date13.target P D []) := by
    intro h
    have hh := (alias_run_coverage P D [] pre (Date13.target P D [] a)).1 h
    simpa using hfirst (by simpa using hh)
  have hd : ¬acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a) = true :=
    fun h => hn ((alias_duplicate P D acc a).1 h)
  have he : uniqueNames P D (pre ++ a :: post) = aliasRun P D (acc ++ [a]) post := by
    change aliasRun P D [] (pre ++ a :: post) = _
    rw [alias_run_append]
    change aliasRun P D
      (if acc.any (fun b => Date13.target P D [] b == Date13.target P D [] a)
       then acc else acc ++ [a]) post = _
    rw [if_neg hd]
  rw [he]
  obtain ⟨tail,ht⟩ := alias_run_prefix P D (acc ++ [a]) post
  rw [ht]
  simp

end Cut
end CM
