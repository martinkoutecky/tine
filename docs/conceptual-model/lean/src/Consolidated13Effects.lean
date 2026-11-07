/- Universal recomputed effects. No transport bundle or desired preservation
is a premise: all readings use the actual output, including its active owners. -/
import Presentation13

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

/-- The output assembled independently by retained-file projection. -/
def assembled (E : Engine) (G : Graph) (old : String) (c : Candidate) : Graph :=
  (kept P G old c).map (outputFile P E G old c)

theorem assembled_eq (E : Engine) (G : Graph) (old : String) (c : Candidate) :
    assembled P E G old c = execute P E G old c := (execute_map_eq P E G old c).symm

/-- An edge exists exactly when an active resulting file declares it. -/
theorem after_alias_edges (E : Engine) (G : Graph) (old : String) (c : Candidate) :
    (readParams P (execute P E G old c)).aliasEdges (activeGraph P (execute P E G old c)) =
    (readParams P (assembled P E G old c)).aliasEdges (activeGraph P (assembled P E G old c)) := by
  rw [assembled_eq]

theorem after_alias_edge_exact (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (a b : K) :
    (a, b) ∈ (readParams P (execute P E G old c)).aliasEdges (activeGraph P (execute P E G old c)) ↔
      ∃ f ∈ (readParams P (assembled P E G old c)).declaringOwners (activeGraph P (assembled P E G old c)),
        (P.claim f).2 = a ∧ ∃ s ∈ P.aliasDecl f.format f.text, P.key s = b := by
  rw [assembled_eq]
  exact Params.mem_aliasEdges _

/-- Complete namespace recomputation, including activation by appended content. -/
theorem merged_namespace_support (J : Merger) (m : String → String) (S D : File) :
    P.fileNsNames (J.apply m S D) =
      if P.inputNonblank (J.apply m S D).text && P.nameIsNamespace (P.name (J.apply m S D))
      then namePrefixes (P.name (J.apply m S D)) else [] := rfl

theorem after_reference_exact (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (b : BlockId) (t : Page K) :
    referencesB P (execute P E G old c) b t = true ↔
      ∃ s ∈ (readParams P (assembled P E G old c)).refNames (activeGraph P (assembled P E G old c)) b,
        Reach ((readParams P (assembled P E G old c)).aliasEdges (activeGraph P (assembled P E G old c)))
          (P.key s) t.2 := by
  rw [assembled_eq]
  simp [referencesB, keyReferences, List.any_eq_true, sameCls_iff, cls, readParams, physicalParams]

theorem after_active_exact (E : Engine) (G : Graph) (old : String) (c : Candidate) (f' : File) :
    f' ∈ activeGraph P (execute P E G old c) ↔
      ∃ f ∈ kept P G old c, outputFile P E G old c f = f' ∧
        openingFile P (assembled P E G old c) (P.claim f').2 = some f' := by
  rw [active_iff, ← assembled_eq]
  simp only [assembled, List.mem_map]
  constructor
  · rintro ⟨⟨f, hf, he⟩, ha⟩; exact ⟨f, hf, he, ha⟩
  · rintro ⟨f, hf, he, ha⟩; exact ⟨⟨f, hf, he⟩, ha⟩

/-- A conflict's names cannot enter the matched domain of a safe operation. -/
theorem safe_conflict_no_match {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P SP E G extras old c = true)
    {f : File} (hf : f ∈ G) (hc : P.conflictedB G (P.claim f).2 = true) :
    nsMatch P.key P.urlHasOrigin old (P.name f) = none := by
  cases hm : nsMatch P.key P.urlHasOrigin old (P.name f) with
  | none => rfl
  | some i =>
    have hd : P.name f ∈ directNames P SP G := by
      apply List.mem_flatMap.mpr
      exact ⟨f, hf, List.mem_cons_self ..⟩
    have hn : (P.claim f).2 ∈ touched P SP G extras old c := by
      unfold touched
      apply List.mem_flatMap.mpr
      refine ⟨P.name f, ?_, List.mem_cons_self ..⟩
      apply List.mem_cons_of_mem
      apply List.mem_cons_of_mem
      apply List.mem_filter.mpr
      refine ⟨List.mem_append_left _ (List.mem_append_right _ hd), ?_⟩
      simp [hm]
    have he := safe_local_conflict P SP h hn
    rw [hc] at he
    contradiction

theorem realized_conflict_path {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece}
    (h : safe P SP E G extras old c = true) (F : Realizes P SP Q E G old c input)
    {f : File} (hf : f ∈ G) (hc : P.conflictedB G (P.claim f).2 = true) :
    ((rewriter E (subst P old c)).apply (subst P old c) f).path = f.path := by
  apply F.rewrites.path f hf
  apply Or.inr
  have hn := safe_conflict_no_match P SP h hf hc
  simp [subst, nsSubst, hn]

theorem realized_conflict_key {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece}
    (h : safe P SP E G extras old c = true) (F : Realizes P SP Q E G old c input)
    {f : File} (hf : f ∈ G) (hc : P.conflictedB G (P.claim f).2 = true) :
    P.key (P.name ((rewriter E (subst P old c)).apply (subst P old c) f)) = P.key (P.name f) := by
  rw [F.rewrites.name f hf]
  have hn := safe_conflict_no_match P SP h hf hc
  simp [subst, nsSubst, hn]

theorem stationary_twins_order {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece}
    (h : safe P SP E G extras old c = true) (F : Realizes P SP Q E G old c input)
    {f g : File} (hf : f ∈ G) (hg : g ∈ G) (hfg : f.path ≠ g.path)
    (hc : P.conflictedB G (P.claim f).2 = true) (he : (P.claim f).2 = (P.claim g).2) :
    claimantLe P ((rewriter E (subst P old c)).apply (subst P old c) f)
      ((rewriter E (subst P old c)).apply (subst P old c) g) = claimantLe P f g := by
  apply claimantLe_distinct_paths P
  · exact realized_conflict_path P SP h F hf hc
  · exact realized_conflict_path P SP h F hg (he ▸ hc)
  · exact hfg
  · exact F.rewrites.dir f hf
  · exact F.rewrites.dir g hg

/-- Expected alias keys from original declarations. The merged header is a
key union; other files retain the exact ordered rewritten declaration list. -/
def aliasKeysFrom (G : Graph) (old : String) (c : Candidate) (f : File) : List K :=
  let m := subst P old c
  match source P G old, c.survivor with
  | some S, some D => if f = D then
      (P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text).map
        (fun s => transport (demands P SP G [] old c) (P.key s))
    else (P.aliasDecl f.format f.text).map (fun s => P.key (m s))
  | _, _ => (P.aliasDecl f.format f.text).map (fun s => P.key (m s))

theorem realized_output_alias {Q : QParams K L} {E : Engine} {G : Graph}
    {old : String} {c : Candidate} {input : File → List Piece}
    (F : Realizes P SP Q E G old c input) {f : File} (hf : f ∈ G) (k : K) :
    (∃ s ∈ P.aliasDecl (outputFile P E G old c f).format (outputFile P E G old c f).text, P.key s = k) ↔
      k ∈ aliasKeysFrom P SP G old c f := by
  have hR : (∃ s ∈ P.aliasDecl ((rewriter E (subst P old c)).apply (subst P old c) f).format
      ((rewriter E (subst P old c)).apply (subst P old c) f).text, P.key s = k) ↔
      k ∈ (P.aliasDecl f.format f.text).map (fun s => P.key (subst P old c s)) := by
    rw [F.rewrites.alias f hf]
    simp only [List.mem_map]
    constructor
    · rintro ⟨s, ⟨s0, hs0, he⟩, hk⟩
      exact ⟨s0, hs0, (congrArg P.key he).trans hk⟩
    · rintro ⟨s0, hs0, hk⟩
      exact ⟨subst P old c s0, ⟨s0, hs0, rfl⟩, hk⟩
  cases hs : source P G old with
  | none => simpa [outputFile, aliasKeysFrom, hs] using hR
  | some S =>
    cases hd : c.survivor with
    | none => simpa [outputFile, aliasKeysFrom, hs, hd] using hR
    | some D =>
      by_cases he : f = D
      · subst f
        simp only [outputFile, aliasKeysFrom, hs, hd, ite_true, List.mem_map]
        have ML := (F.merge S D hs hd).1
        constructor
        · rintro ⟨s, hs', hk⟩
          obtain ⟨s0, hs0, he⟩ := ML.aliasL.1 s hs'
          exact ⟨s0, hs0, he.trans hk⟩
        · rintro ⟨s0, hs0, hk⟩
          obtain ⟨s, hs', he⟩ := ML.aliasL.2 s0 hs0
          exact ⟨s, hs', he.trans hk⟩
      · simpa [outputFile, aliasKeysFrom, hs, hd, he] using hR

def added {α : Type} [DecidableEq α] (before after : List α) : List α :=
  after.eraseDups.filter (fun x => !before.contains x)
def removed {α : Type} [DecidableEq α] (before after : List α) : List α := added after before

theorem added_exact {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    x ∈ added A B ↔ x ∈ B ∧ x ∉ A := by simp [added]

theorem removed_exact {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    x ∈ removed A B ↔ x ∈ A ∧ x ∉ B := by simp [removed, added]

theorem changes_complete {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    (x ∈ A ↔ x ∈ B) ↔ x ∉ added A B ∧ x ∉ removed A B := by
  rw [added_exact, removed_exact]
  constructor
  · intro h; exact ⟨fun ⟨hb, ha⟩ => ha (h.mpr hb), fun ⟨ha, hb⟩ => hb (h.mp ha)⟩
  · rintro ⟨hn, hm⟩
    constructor
    · intro ha; by_cases hb : x ∈ B; exact hb; exact False.elim (hm ⟨ha, hb⟩)
    · intro hb; by_cases ha : x ∈ A; exact ha; exact False.elim (hn ⟨hb, ha⟩)

theorem linked_exact (favorites : Option String) (G : Graph) (b : BlockId) (t : Page K) :
    b ∈ linked P favorites G t ↔ b ∈ blocks P G ∧ referencesB P G b t = true ∧
      panelSource P favorites G t b = true := by simp [linked, relationHits, and_comm]

theorem after_linked_exact (favorites : Option String) (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (b : BlockId) (t : Page K) :
    b ∈ linked P favorites (execute P E G old c) t ↔
      b ∈ blocks P (assembled P E G old c) ∧
      (∃ s ∈ (readParams P (assembled P E G old c)).refNames (activeGraph P (assembled P E G old c)) b,
        Reach ((readParams P (assembled P E G old c)).aliasEdges (activeGraph P (assembled P E G old c)))
          (P.key s) t.2) ∧ panelSource P favorites (assembled P E G old c) t b = true := by
  rw [linked_exact, after_reference_exact, assembled_eq]

theorem length_partition {α : Type} (l : List α) (p : α → Bool) :
    l.length = (l.filter p).length + (l.filter (fun x => !p x)).length := by
  induction l with
  | nil => simp
  | cons x xs ih => cases hp : p x <;> simp [hp, ih, Nat.add_assoc, Nat.add_comm, Nat.add_left_comm]

/-- Counts include exactly the retained and gained active blocks, with identity
comparison in the two finite block universes; no injective block transport is needed. -/
theorem after_count_exact (favorites : Option String) (E : Engine) (G : Graph) (old : String) (c : Candidate) (t : Page K) :
    referenceCount P favorites (execute P E G old c) t =
      ((linked P favorites (execute P E G old c) t).filter (fun b => (linked P favorites G t).contains b)).length +
      ((linked P favorites (execute P E G old c) t).filter (fun b => !(linked P favorites G t).contains b)).length :=
  length_partition _ _

/-- Exact query membership after any candidate, for arbitrary Boolean queries
and supplied predicates. In particular the argument can be renameQuery m q. -/
theorem after_query_exact (Q : QParams K L) (E : Engine) (G : Graph) (old : String)
    (c : Candidate) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ den P Q (execute P E G old c) a q ↔
      n ∈ (readParams P (assembled P E G old c)).univ true (activeGraph P (assembled P E G old c)) a ∧
      q.bval (fun l => (readParams P (assembled P E G old c)).leafAt3 Q
        (activeGraph P (assembled P E G old c)) l n) = true := by
  rw [assembled_eq]
  exact (Params.mem_denU3 _ Q)

/-- Stored argument changes and result changes are distinct. This exact result
set works for negation, joins and opaque leaves without any preservation premise. -/
theorem query_gained_exact (Q : QParams K L) (E : Engine) (G : Graph) (old : String)
    (c : Candidate) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ added (den P Q G a q) (den P Q (execute P E G old c) a (renameQuery (subst P old c) q)) ↔
      (n ∈ (readParams P (assembled P E G old c)).univ true (activeGraph P (assembled P E G old c)) a ∧
      (renameQuery (subst P old c) q).bval (fun l =>
        (readParams P (assembled P E G old c)).leafAt3 Q (activeGraph P (assembled P E G old c)) l n) = true) ∧
      n ∉ den P Q G a q := by
  rw [added_exact, after_query_exact]

/-- Presentation is recomputed from the resulting raw query, including raw parent hits. -/
theorem after_queryResults_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (E : Engine) (G : Graph) (old : String) (c : Candidate) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P Q R o (execute P E G old c) a q ↔
      (n ∈ (readParams P (assembled P E G old c)).univ true (activeGraph P (assembled P E G old c)) a ∧
        q.bval (fun l => (readParams P (assembled P E G old c)).leafAt3 Q
          (activeGraph P (assembled P E G old c)) l n) = true) ∧
      queryRowVisible P Q R o (assembled P E G old c)
        (den P Q (assembled P E G old c) a q) n = true := by
  rw [queryResults_exact, after_query_exact, assembled_eq]

theorem queryResults_gained_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (E : Engine) (G : Graph) (old : String) (c : Candidate) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ added (queryResults P Q R o G a q)
      (queryResults P Q R o (execute P E G old c) a (renameQuery (subst P old c) q)) ↔
      (n ∈ den P Q (execute P E G old c) a (renameQuery (subst P old c) q) ∧
        queryRowVisible P Q R o (execute P E G old c)
          (den P Q (execute P E G old c) a (renameQuery (subst P old c) q)) n = true) ∧
      n ∉ queryResults P Q R o G a q := by
  rw [added_exact, queryResults_exact]

theorem after_links_exact (config : Decided.Config) (E : Engine) (G : Graph)
    (old : String) (c : Candidate) (S T : Page K) :
    links P config (execute P E G old c) S T = true ↔
      S ∈ pages P (assembled P E G old c) ∧ T ∈ pages P (assembled P E G old c) ∧ S ≠ T ∧
      S.2 ≠ P.key config.favoritesPage ∧
      ∃ b ∈ (readParams P (assembled P E G old c)).pageBlocks (activeGraph P (assembled P E G old c)) S,
        referencesB P (assembled P E G old c) b T = true := by
  rw [assembled_eq]
  simp [links, Bool.and_eq_true, List.any_eq_true, and_assoc]

/-- The universal operation theorem rewrites the whole reader to the actual
assembled output. The more useful membership/change equations above unfold it. -/
theorem offered_recomputed (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (R : QueryRowParser) (o : QueryPresentation) (E : Engine)
    {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : c ∈ offers P SP E G extras old new) :
    ∃ H, answer P SP E G extras old new (some c) = some H ∧
      H = assembled P E G old c ∧
      referencesB P H = referencesB P (assembled P E G old c) ∧
      linked P favorites H = linked P favorites (assembled P E G old c) ∧
      referenceCount P favorites H = referenceCount P favorites (assembled P E G old c) ∧
      den P Q H = den P Q (assembled P E G old c) ∧
      queryResults P Q R o H = queryResults P Q R o (assembled P E G old c) ∧
      links P config H = links P config (assembled P E G old c) ∧
      resolve P H = resolve P (assembled P E G old c) := by
  refine ⟨execute P E G old c, offered_executes P SP h, ?_⟩
  rw [assembled_eq]
  exact ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

end Consolidated13
end CM
