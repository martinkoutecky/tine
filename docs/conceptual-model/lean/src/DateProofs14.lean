import DateOperations14

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6 Merge7
variable {K L : Type} [DecidableEq K] (P : Inputs K) (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L)

theorem memberSpelling_identifies (f : File) :
    key P dates (memberSpelling P dates N f) = claim P dates f := by
  unfold memberSpelling
  split
  · assumption
  · exact N.identifies f

/-- No display-label premise: every distinct real group member is enumerated. -/
theorem group_member_candidate {G : Graph} {old new : String} {f : File}
    (hf : f ∈ groupFiles P dates G new) (hne : claim P dates f ≠ key P dates old) :
    literal P dates M G old (memberSpelling P dates N f) ∈ candidates P dates M N G old new := by
  simp only [candidates, List.mem_eraseDups, List.mem_cons]
  exact Or.inr (List.mem_map.mpr ⟨f, List.mem_filter.mpr ⟨hf, by simpa using hne⟩, rfl⟩)

/-- The selected member's real identity is the destination of the ONE map. -/
theorem group_member_substitution (G : Graph) (old : String) (f : File) :
    key P dates (literal P dates M G old (memberSpelling P dates N f)).destination = claim P dates f ∧
    subst P dates M old (literal P dates M G old (memberSpelling P dates N f)) =
      nsSubst (key P dates) P.urlHasOrigin old (memberSpelling P dates N f) := by
  exact ⟨memberSpelling_identifies P dates N f, rfl⟩

theorem offered_safe {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∈ offers P dates M N SP E G extras old new) :
    safe P dates M SP E G extras old c = true := (List.mem_filter.mp h).2

theorem offers_exact {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} : c ∈ offers P dates M N SP E G extras old new ↔
      c ∈ candidates P dates M N G old new ∧ safe P dates M SP E G extras old c = true := by
  exact List.mem_filter

theorem default_offered {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : defaultCandidate P dates M SP E G extras old new = some c) :
    c ∈ offers P dates M N SP E G extras old new := by
  change (if safe P dates M SP E G extras old (literal P dates M G old new) then
    some (literal P dates M G old new) else none) = some c at h
  split at h
  · next hs =>
    cases Option.some.inj h
    apply List.mem_filter.mpr
    exact ⟨by simp [candidates], hs⟩
  · contradiction

/-- Comprehensive group choice, beyond filtered-list completeness. Safety is
the existing finite guard; no identifying-displayed-label condition is needed. -/
theorem safe_group_member_offered {E : Engine} {G : Graph} {extras : List String}
    {old new : String} {f : File} (hf : f ∈ groupFiles P dates G new)
    (hne : claim P dates f ≠ key P dates old)
    (hs : safe P dates M SP E G extras old (literal P dates M G old (memberSpelling P dates N f)) = true) :
    literal P dates M G old (memberSpelling P dates N f) ∈ offers P dates M N SP E G extras old new :=
  List.mem_filter.mpr ⟨group_member_candidate P dates M N hf hne, hs⟩

/-- Semantic completeness and soundness: the literal option or a real group
member's checked identifying spelling, with exactly the shared safety guard. -/
theorem group_choices_exact {E : Engine} {G : Graph} {extras : List String}
    {old new : String} {c : Candidate} :
    c ∈ offers P dates M N SP E G extras old new ↔
      (c = literal P dates M G old new ∨
        ∃ f ∈ groupFiles P dates G new, claim P dates f ≠ key P dates old ∧
          c = literal P dates M G old (memberSpelling P dates N f)) ∧
      safe P dates M SP E G extras old c = true := by
  simp only [offers, candidates, List.mem_filter, List.mem_eraseDups,
    List.mem_cons, List.mem_map, decide_eq_true_eq]
  constructor
  · rintro ⟨h, hs⟩
    refine ⟨?_, hs⟩
    rcases h with h | ⟨f, ⟨hf, hn⟩, he⟩
    · exact Or.inl h
    · exact Or.inr ⟨f, hf, hn, he.symm⟩
  · rintro ⟨h, hs⟩
    refine ⟨?_, hs⟩
    rcases h with h | ⟨f, hf, hn, he⟩
    · exact Or.inl h
    · exact Or.inr ⟨f, ⟨hf, hn⟩, he.symm⟩

theorem safe_consistent {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) :
    consistent (demands P dates M SP G extras old c) = true := by
  simp only [safe, Bool.and_eq_true] at h
  exact h.1.1.1.1.1.1.2

theorem safe_wf {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) :
    WF G ∧ WF (execute P dates M E G old c) := by
  simp only [safe, Bool.and_eq_true, decide_eq_true_eq] at h
  exact ⟨h.1.2, h.2⟩

theorem safe_root {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) :
    transport (demands P dates M SP G extras old c) (key P dates old) = key P dates c.destination := by
  exact transport_realizes (safe_consistent P dates M SP h) (a := (key P dates old, key P dates c.destination))
    (List.mem_cons_self ..)

theorem safe_anchor {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) {t : Page K}
    (ht : t ∈ anchors P dates M G (old :: directNames P dates M SP G ++ extras)) :
    transport (demands P dates M SP G extras old c) t = t := by
  apply transport_realizes (safe_consistent P dates M SP h) (a := (t,t))
  unfold demands
  exact List.mem_cons_of_mem _ (List.mem_append_right _ (List.mem_map.mpr ⟨t,ht,rfl⟩))

theorem safe_site {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) {s : String}
    (hs : s ∈ directNames P dates M SP G ++ extras) :
    transport (demands P dates M SP G extras old c) (key P dates s) = key P dates (subst P dates M old c s) := by
  apply transport_realizes (safe_consistent P dates M SP h)
    (a := (key P dates s, key P dates (subst P dates M old c s)))
  unfold demands
  exact List.mem_cons_of_mem _ (List.mem_append_left _ (List.mem_map.mpr ⟨s, hs, rfl⟩))

theorem cancel_identity (E : Engine) (G : Graph) (extras : List String) (old new : String) :
    answer P dates M N SP E G extras old new none = some G := rfl

theorem unoffered_refused {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∉ offers P dates M N SP E G extras old new) :
    answer P dates M N SP E G extras old new (some c) = none := by simp [answer, h]

theorem offered_executes {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∈ offers P dates M N SP E G extras old new) :
    answer P dates M N SP E G extras old new (some c) = some (execute P dates M E G old c) := by
  simp [answer, h]

theorem safe_group_member_reachable {E : Engine} {G : Graph} {extras : List String}
    {old new : String} {f : File} (hf : f ∈ groupFiles P dates G new)
    (hne : claim P dates f ≠ key P dates old)
    (hs : safe P dates M SP E G extras old (literal P dates M G old (memberSpelling P dates N f)) = true) :
    answer P dates M N SP E G extras old new
      (some (literal P dates M G old (memberSpelling P dates N f))) =
        some (execute P dates M E G old (literal P dates M G old (memberSpelling P dates N f))) :=
  offered_executes P dates M N SP (safe_group_member_offered P dates M N SP hf hne hs)

theorem automatic_unique {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {G' : Graph} (h : automatic P dates M N SP E G extras old new = some G') :
    ∃ c, offers P dates M N SP E G extras old new = [c] ∧ G' = execute P dates M E G old c := by
  unfold automatic at h
  split at h
  · next c he => exact ⟨c, he, Option.some.inj h |>.symm⟩
  · contradiction

theorem fileless_cardinality (E : Engine) {G : Graph} {old : String} (c : Candidate)
    (h : source P dates M G old = none) : (execute P dates M E G old c).length = G.length := by
  simp [execute, h, renameBy]

theorem merge_exact_files (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {S D : File} (hs : source P dates M G old = some S) (hd : c.survivor = some D) :
    execute P dates M E G old c = (G.filter (fun f => decide (f ≠ S))).map
      (fun f => if f = D then (E.merger (subst P dates M old c)).apply (subst P dates M old c) S D
        else (rewriter E (subst P dates M old c)).apply (subst P dates M old c) f) := by
  simp [execute, hs, hd, mergeBy, Decided.Rewriter.base]


theorem singleton_executes {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : offers P dates M N SP E G extras old new = [c]) :
    automatic P dates M N SP E G extras old new = some (execute P dates M E G old c) := by
  simp [automatic,h]

theorem distinct_offers_block_automatic {E : Engine} {G : Graph} {extras : List String}
    {old new : String} {a b : Candidate}
    (ha : a ∈ offers P dates M N SP E G extras old new)
    (hb : b ∈ offers P dates M N SP E G extras old new) (hne : a ≠ b) :
    automatic P dates M N SP E G extras old new = none := by
  cases h : automatic P dates M N SP E G extras old new with
  | none => rfl
  | some H =>
    obtain ⟨c, hc, _⟩ := automatic_unique P dates M N SP h
    rw [hc] at ha hb
    simp only [List.mem_singleton] at ha hb
    exact False.elim (hne (ha.trans hb.symm))

theorem safe_no_new_claim_collision {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P dates M SP E G extras old c = true)
    {f g : File} (hf : f ∈ kept P dates M G old c) (hg : g ∈ kept P dates M G old c)
    (he : claim P dates (outputFile P dates M E G old c f) = claim P dates (outputFile P dates M E G old c g)) :
    claim P dates f = claim P dates g := by
  have ho : ownership P dates M E G old c = true := by
    simp only [safe,Bool.and_eq_true] at h
    exact h.1.1.1.2
  have hh := List.all_eq_true.mp (List.all_eq_true.mp ho f hf) g hg
  exact (of_decide_eq_true hh) he

theorem safe_local_conflict {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P dates M SP E G extras old c = true)
    {t : Page K} (ht : t ∈ touched P dates M SP G extras old c) : conflicted P dates G t = false := by
  have hc : localClean P dates M SP G extras old c = true := by
    simp only [safe,Bool.and_eq_true] at h
    exact h.1.1.1.1.1.2
  have hh := List.all_eq_true.mp hc t ht
  cases he : conflicted P dates G t <;> simp_all

/-- Exact assembly: same retained-file map as the earlier library, now using day claims. -/
theorem execute_map (E : Engine) (G : Graph) (old : String) (c : Candidate) :
    execute P dates M E G old c = (kept P dates M G old c).map (outputFile P dates M E G old c) := by
  have ht : G.filter (fun _ => true) = G := by induction G <;> simp_all
  unfold execute kept outputFile
  cases hs : source P dates M G old <;> cases hd : c.survivor <;>
    simp [hs,hd,mergeBy,renameBy,Decided.Rewriter.base,eq_comm,ht]

end Date13
end CM
