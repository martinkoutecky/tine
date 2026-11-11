import DateSafety14

set_option linter.unusedSectionVars false
set_option maxHeartbeats 1600000
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L)

theorem conflicted_exact (G : Graph) (t : Page K) : conflicted P dates G t = true ↔
    ∃ f ∈ G, ∃ g ∈ G, f ≠ g ∧ claim P dates f = t ∧ claim P dates g = t := by simp [conflicted]

theorem uniqueClaim_spec {G : Graph} {k : Page K} {f : File}
    (h : uniqueClaim P dates M G k = some f) :
    f ∈ G ∧ claim P dates f = k ∧ ∀ g ∈ G, claim P dates g = k → g = f := by
  unfold uniqueClaim at h
  generalize he : G.filter (fun f => decide (claim P dates f = k)) = fs at h
  cases fs with
  | nil => simp at h
  | cons a as =>
    cases as with
    | cons b bs => simp at h
    | nil =>
      simp only [Option.some.injEq] at h
      subst a
      have hm : f ∈ G ∧ claim P dates f = k := by
        have : f ∈ G.filter (fun f => decide (claim P dates f = k)) := by rw [he]; simp
        simpa using this
      refine ⟨hm.1, hm.2, ?_⟩
      intro g hg hk
      have : g ∈ G.filter (fun f => decide (claim P dates f = k)) := by simp [hg, hk]
      simpa [he] using this

theorem uniqueClaim_not_conflicted {G : Graph} {k : Page K} {f : File}
    (h : uniqueClaim P dates M G k = some f) : conflicted P dates G k = false := by
  have hu := (uniqueClaim_spec P dates M h).2.2
  cases hc : conflicted P dates G k
  · rfl
  · obtain ⟨a, ha, b, hb, hn, hak, hbk⟩ := (conflicted_exact P dates G _).mp hc
    exact False.elim (hn ((hu a ha hak).trans (hu b hb hbk).symm))

theorem conflict_not_unique {G : Graph} {f : File} {k : Page K}
    (hc : conflicted P dates G (claim P dates f) = true) : uniqueClaim P dates M G k ≠ some f := by
  intro h
  have hk := (uniqueClaim_spec P dates M h).2.1
  have hn := uniqueClaim_not_conflicted P dates M h
  rw [← hk, hc] at hn
  contradiction

theorem conflict_not_source {G : Graph} {f : File} {old : String}
    (hc : conflicted P dates G (claim P dates f) = true) : source P dates M G old ≠ some f := by
  intro h
  unfold source at h
  exact conflict_not_unique P dates M hc (Option.filter_eq_some_iff.mp h).1

theorem safe_pairReady {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) : pairReady P dates M G old c = true := by
  simp only [safe, Bool.and_eq_true] at h
  exact h.1.1.1.1.2

theorem conflict_not_survivor {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P dates M SP E G extras old c = true)
    {f : File} (hc : conflicted P dates G (claim P dates f) = true) : c.survivor ≠ some f := by
  intro hd
  have hp := safe_pairReady P dates M SP h
  unfold pairReady at hp
  rw [hd] at hp
  cases hs : source P dates M G old with
  | none => simp [hs] at hp
  | some S =>
    simp only [hs, Bool.and_eq_true, decide_eq_true_eq] at hp
    exact conflict_not_unique P dates M hc hp.1.2

theorem conflict_kept {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (_h : safe P dates M SP E G extras old c = true)
    {f : File} (hf : f ∈ G) (hc : conflicted P dates G (claim P dates f) = true) :
    f ∈ kept P dates M G old c := by
  have hn := conflict_not_source P dates M (old := old) hc
  unfold kept
  split
  · exact List.mem_filter.mpr ⟨hf, decide_eq_true hn⟩
  · exact hf

theorem conflict_output {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P dates M SP E G extras old c = true)
    {f : File} (hc : conflicted P dates G (claim P dates f) = true) :
    outputFile P dates M E G old c f = (rewriter E (subst P dates M old c)).apply (subst P dates M old c) f := by
  have hn := conflict_not_survivor P dates M SP h hc
  cases hs : source P dates M G old with
  | none => simp only [outputFile, hs]
  | some S =>
    cases hd : c.survivor with
    | none => simp only [outputFile, hs, hd]
    | some D =>
      have hne : f ≠ D := by
        intro he
        subst D
        exact hn hd
      simp only [outputFile, hs, hd, if_neg hne]

theorem kept_subset (G : Graph) (old : String) (c : Candidate) :
    ∀ f ∈ kept P dates M G old c, f ∈ G := by
  intro f hf
  unfold kept at hf
  split at hf
  · exact (List.mem_filter.mp hf).1
  · exact hf

theorem safe_conflict_no_match {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P dates M SP E G extras old c = true) {f : File} (hf : f ∈ G)
    (hc : conflicted P dates G (claim P dates f) = true) (hd : f.dir = .pages) :
    nsMatch (key P dates) P.urlHasOrigin old (P.name f) = none := by
  cases hm : nsMatch (key P dates) P.urlHasOrigin old (P.name f) with
  | none => rfl
  | some i =>
    have hn : claim P dates f ∈ touched P dates M SP G extras old c := by
      unfold touched
      apply List.mem_flatMap.mpr
      refine ⟨P.name f,?_,?_⟩
      · apply List.mem_cons_of_mem
        apply List.mem_cons_of_mem
        apply List.mem_filter.mpr
        refine ⟨List.mem_append_left _ (List.mem_append_right _ ?_),by simp [hm]⟩
        exact List.mem_flatMap.mpr ⟨f,hf,List.mem_cons_self ..⟩
      · have he : claim P dates f = key P dates (P.name f) := by simp [claim,key,hd]
        rw [he]; exact List.mem_cons_self ..
    have he := safe_local_conflict P dates M SP h hn
    rw [hc] at he; contradiction

theorem realized_conflict_path {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role}
    (h : safe P dates M SP E G extras old c = true) (F : Realizes P dates M SP Q A E G old c input role)
    {f : File} (hf : f ∈ G) (hc : conflicted P dates G (claim P dates f) = true) :
    ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f).path = f.path := by
  apply F.rewrites.path f hf
  cases hd : f.dir with
  | journals => exact Or.inl (by simp [kindOf,hd])
  | pages =>
    right
    have hn := safe_conflict_no_match P dates M SP h hf hc hd
    simp [subst,nsSubst,hn]

theorem realized_conflict_key {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role}
    (h : safe P dates M SP E G extras old c = true) (F : Realizes P dates M SP Q A E G old c input role)
    {f : File} (hf : f ∈ G) (hc : conflicted P dates G (claim P dates f) = true) :
    claim P dates ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f) = claim P dates f := by
  apply claim_of_same_name_path P dates (F.rewrites.dir f hf) (realized_conflict_path P dates M SP h F hf hc)
  rw [F.rewrites.name f hf]
  cases hd : f.dir with
  | journals => simp [kindOf,hd]
  | pages =>
    have hn := safe_conflict_no_match P dates M SP h hf hc hd
    simp [kindOf,hd,subst,nsSubst,hn]

theorem stationary_twins_order {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role}
    (h : safe P dates M SP E G extras old c = true) (F : Realizes P dates M SP Q A E G old c input role)
    {f g : File} (hf : f ∈ G) (hg : g ∈ G) (hne : f.path ≠ g.path)
    (hc : conflicted P dates G (claim P dates f) = true) (he : claim P dates f = claim P dates g) :
    fileLe ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f)
      ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) g) = fileLe f g := by
  have hn : Owner5.bytes f.path ≠ Owner5.bytes g.path := fun h => hne (Owner5.bytes_injective h)
  simp [fileLe,pathLe,Owner5.rankLe,realized_conflict_path P dates M SP h F hf hc,
    realized_conflict_path P dates M SP h F hg (he ▸ hc),F.rewrites.dir f hf,F.rewrites.dir g hg,hn]

/-- Full ownership theorem: the same physical path opens after any safe
realized candidate at any previously conflicted key. No preservation premise. -/
theorem stationary_twins_opening {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role} {A : AliasParser}
    (h : safe P dates M SP E G extras old c = true) (F : Realizes P dates M SP Q A E G old c input role)
    {k : Page K} (hc : conflicted P dates G k = true) :
    (openingFile P dates (execute P dates M E G old c) k).map File.path =
      (openingFile P dates G k).map File.path := by
  let r := fun f => (rewriter E (subst P dates M old c)).apply (subst P dates M old c) f
  let fs := G.filter (fun f => decide (claim P dates f = k))
  have facts : ∀ f ∈ fs, f ∈ kept P dates M G old c ∧ outputFile P dates M E G old c f = r f ∧
      claim P dates (r f) = k ∧ (r f).path = f.path := by
    intro f hf
    obtain ⟨hf, hk⟩ := List.mem_filter.mp hf
    have hk : claim P dates f = k := of_decide_eq_true hk
    have hc' : conflicted P dates G (claim P dates f) = true := hk ▸ hc
    exact ⟨conflict_kept P dates M SP h hf hc', conflict_output P dates M SP h hc',
      (realized_conflict_key P dates M SP h F hf hc').trans hk,
      realized_conflict_path P dates M SP h F hf hc'⟩
  have same : ∀ x, x ∈ (execute P dates M E G old c).filter (fun f => decide (claim P dates f = k)) ↔
      x ∈ fs.map r := by
    intro x
    simp only [List.mem_filter, decide_eq_true_eq, List.mem_map]
    constructor
    · rintro ⟨hx, hk⟩
      have hm := execute_map P dates M E G old c
      rw [hm] at hx
      obtain ⟨g, hg, rfl⟩ := List.mem_map.mp hx
      obtain ⟨a, ha, b, hb, _, hak, _⟩ := (conflicted_exact P dates G _).mp hc
      have ha' : a ∈ fs := List.mem_filter.mpr ⟨ha, decide_eq_true hak⟩
      have fa := facts a ha'
      have hgk := safe_no_new_claim_collision P dates M SP h hg fa.1 (hk.trans (fa.2.1 ▸ fa.2.2.1).symm)
      have hgf : g ∈ fs := List.mem_filter.mpr
        ⟨kept_subset P dates M G old c g hg, decide_eq_true (hgk.trans hak)⟩
      exact ⟨g, hgf, (facts g hgf).2.1.symm⟩
    · rintro ⟨f, hf, rfl⟩
      have ff := facts f hf
      constructor
      · have hm := execute_map P dates M E G old c
        rw [hm, ← ff.2.1]
        exact List.mem_map_of_mem ff.1
      · exact ff.2.2.1
  have order : ∀ a ∈ fs, ∀ b ∈ fs, fileLe (r a) (r b) = fileLe a b := by
    intro a ha b hb
    obtain ⟨haG, hak⟩ := List.mem_filter.mp ha
    obtain ⟨hbG, hbk⟩ := List.mem_filter.mp hb
    have hak : claim P dates a = k := of_decide_eq_true hak
    have hbk : claim P dates b = k := of_decide_eq_true hbk
    by_cases he : a.path = b.path
    · have he' := wf_path_injective (safe_wf P dates M SP h).1 haG hbG he
      subst b
      have refl := fun f => (fileLe_linear).total f f |>.elim id id
      rw [refl, refl]
    · exact stationary_twins_order P dates M SP h F haG hbG he (hak ▸ hc) (hak.trans hbk.symm)
  unfold openingFile claimants
  rw [minBy_congr (fileLe_linear) same, minBy_map_order _ _ r fs order]
  cases hm : minBy fileLe fs with
  | none => rfl
  | some f =>
    exact congrArg some (facts f (minBy_mem hm)).2.2.2

theorem offered_twins_opening {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old new : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role} {A : AliasParser}
    (h : c ∈ offers P dates M N SP E G extras old new) (F : Realizes P dates M SP Q A E G old c input role)
    {k : Page K} (hc : conflicted P dates G k = true) :
    (openingFile P dates (execute P dates M E G old c) k).map File.path =
      (openingFile P dates G k).map File.path :=
  stationary_twins_opening P dates M SP (offered_safe P dates M N SP h) F hc


end Date13
end CM
