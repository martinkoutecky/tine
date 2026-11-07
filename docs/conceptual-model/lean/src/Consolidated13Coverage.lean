/- Exhaustive comparison diagnostics. R12 has choices, not a default; the
comparison is per chosen old option. All refusal causes and primitive-selection
changes are executable and independent of equality of complete after graphs. -/
import Consolidated13Effects

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 Rulings9 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

/-- Remaining finite refusal causes, not desired equality of old/new results. -/
def refusalChecks (E : Engine) (G : Graph) (extras : List String) (old new : String) : List (String × Bool) :=
  let c := literal P G old new
  [("source support", supported P G old),
   ("key consistency", consistent (demands P SP G extras old c)),
   ("touched conflicts", localClean P SP G extras old c),
   ("content pair", pairReady P G old c),
   ("output claims", ownership P E G old c),
   ("virtual capture", noCapture P G old c),
   ("input paths", decide ((G.map File.path).Nodup)),
   ("output paths", decide (((execute P E G old c).map File.path).Nodup))]

theorem refusalChecks_exact (E : Engine) (G : Graph) (extras : List String) (old new : String) :
    (refusalChecks P SP E G extras old new).all Prod.snd =
      safe P SP E G extras old (literal P G old new) := by
  simp [refusalChecks, safe, Bool.and_assoc]

/-- How the old operation is assembled, before running any writer. File-less
sources are substitutions even when the old UI called them merges. -/
def oldPair (G : Graph) (old new : String) : Option (File × File) :=
  if Rulings11.noFileSource P.toParams G old then none else
    (Merge7.mergeSource P.toParams G old).bind (fun S => (Rulings11.target P.toParams G old new).map (S, ·))

def newPair (G : Graph) (old new : String) : Option (File × File) :=
  (source P G old).bind (fun S => (literal P G old new).survivor.map (S, ·))

/-- New flags are physical pair selection, target spelling, and failed finite
checks. Old ownPage is deliberately its own route; it was never R12's default. -/
def comparisonCases (E : Engine) (G : Graph) (extras : List String) (old new : String)
    (choice : Rulings11.Choice) : List (String × Bool) :=
  differenceCases P SP G extras old new ++
  [("legacy own-page route", choice == .ownPage),
   ("rename/content-pair selection", choice == .rename && (newPair P G old new).isSome),
   ("merge/content-pair selection", choice == .mergeOwner && decide (oldPair P G old new ≠ newPair P G old new)),
   ("old target spelling", choice == .mergeOwner &&
      (Rulings11.target P.toParams G old new).any (fun D => decide (P.name D ≠ new)))] ++
  (refusalChecks P SP E G extras old new).map (fun x => ("refused: " ++ x.1, !x.2))

def anyDifference (xs : List (String × Bool)) : Prop := ∃ x ∈ xs, x.2 = true

theorem comparison_refusal (E : Engine) {G : Graph} {extras : List String} {old new : String}
    (choice : Rulings11.Choice)
    (h : safe P SP E G extras old (literal P G old new) = false) :
    anyDifference (comparisonCases P SP E G extras old new choice) := by
  have hn : ∃ x ∈ refusalChecks P SP E G extras old new, x.2 = false := by
    have he := refusalChecks_exact P SP E G extras old new
    rw [h] at he
    simpa only [List.all_eq_false, Bool.not_eq_true] using he
  obtain ⟨x, hx, he⟩ := hn
  refine ⟨("refused: " ++ x.1, !x.2), List.mem_append_right _ (List.mem_map.mpr ⟨x, hx, rfl⟩), ?_⟩
  simp [he]

/-- A map split has a concrete spelling witness. It is not decidable in
full generality because the configured key is an arbitrary String function.
The witness itself is finitely executable. No output-graph inequality is tested. -/
def mapSplit (G : Graph) (old new : String) : Prop :=
  ∃ D, Rulings11.target P.toParams G old new = some D ∧ ∃ s,
    queryMap P.toParams old new D s ≠ nsSubst P.key P.urlHasOrigin old new s

/-- If the pair selectors agree, running the selected primitive agrees. -/
theorem merge_pair_effect (E : Engine) {G H : Graph} {old new : String}
    (h : Rulings11.mergeChosen P.toParams E.rewrite E.merger G old new = some H)
    (hp : oldPair P G old new = newPair P G old new)
    (ht : ¬mapSplit P G old new) :
    execute P E G old (literal P G old new) = H := by
  obtain ⟨D, hd, _, he⟩ := Rulings11.mergeChosen_structure P.toParams E.rewrite E.merger h
  have hm : queryMap P.toParams old new D = nsSubst P.key P.urlHasOrigin old new := by
    funext s
    by_cases h : queryMap P.toParams old new D s = nsSubst P.key P.urlHasOrigin old new s
    · exact h
    · exact False.elim (ht ⟨D, hd, s, h⟩)
  rcases he with ⟨hn, he⟩ | ⟨S, hs, he⟩
  · have hp' : newPair P G old new = none := by simpa [oldPair, hn] using hp.symm
    rw [he, hm]
    unfold execute
    cases hs : source P G old with
    | none => rfl
    | some S =>
      cases hc : (literal P G old new).survivor with
      | none => rfl
      | some D => simp [newPair, hs, hc] at hp'
  · have hn : Rulings11.noFileSource P.toParams G old = false := by
      cases hn : Rulings11.noFileSource P.toParams G old
      · rfl
      · simp [Rulings11.noFileSource, Rulings11.aliasSource_none_of_mergeSource P.toParams hs] at hn
        have hnone := Ref10.mergeSource_none P.toParams hn
        rw [hs] at hnone
        contradiction
    have hp' : newPair P G old new = some (S, D) := by simpa [oldPair, hn, hs, hd] using hp.symm
    unfold newPair at hp'
    cases hsrc : source P G old with
    | none => simp [hsrc] at hp'
    | some S' =>
      cases hdst : (literal P G old new).survivor with
      | none => simp [hsrc, hdst] at hp'
      | some D' =>
        have eqs : S' = S ∧ D' = D := by simpa [hsrc, hdst] using hp'
        obtain ⟨rfl, rfl⟩ := eqs
        rw [he, hm]
        unfold execute
        rw [hsrc, hdst]
        rfl

/-- Every old choice, including file-less merge, has an accounted-for result.
Cancellation is compared to cancellation, rather than to a mutation default. -/
theorem comparison_exhaustive (E : Engine) (G : Graph) (extras : List String)
    (old new : String) (choice : Rulings11.Choice) :
    Rulings11.answer P.toParams E.rewrite E.merger G old new choice = none ∨
    choice = .cancel ∨
    anyDifference (comparisonCases P SP E G extras old new choice) ∨
    (choice = .mergeOwner ∧ mapSplit P G old new) ∨
    ∃ H, defaultCandidate P SP E G extras old new = some (literal P G old new) ∧
      Rulings11.answer P.toParams E.rewrite E.merger G old new choice = some H ∧
      execute P E G old (literal P G old new) = H := by
  classical
  cases ho : Rulings11.answer P.toParams E.rewrite E.merger G old new choice with
  | none => exact Or.inl rfl
  | some H =>
    right
    by_cases hc : choice = .cancel
    · exact Or.inl hc
    right
    cases hs : safe P SP E G extras old (literal P G old new) with
    | false => exact Or.inl (comparison_refusal P SP E choice hs)
    | true =>
      by_cases hx : anyDifference (comparisonCases P SP E G extras old new choice)
      · exact Or.inl hx
      right
      have hmemb : ∀ x ∈ comparisonCases P SP E G extras old new choice, x.2 ≠ true := by
        intro x hx' he; exact hx ⟨x, hx', he⟩
      have hdefault : defaultCandidate P SP E G extras old new = some (literal P G old new) := by
        simp [defaultCandidate, hs]
      cases choice with
      | cancel => exact False.elim (hc rfl)
      | ownPage =>
        exact False.elim (hx ⟨("legacy own-page route", true), by simp [comparisonCases], rfl⟩)
      | rename =>
        apply Or.inr
        refine ⟨H, hdefault, rfl, ?_⟩
        have hp : newPair P G old new = none := by
          cases hpair : newPair P G old new with
          | none => rfl
          | some pair =>
            exact False.elim (hx ⟨("rename/content-pair selection", true), by simp [comparisonCases, hpair], rfl⟩)
        rw [old_rename_shape P E.rewrite E.merger ho]
        unfold execute
        cases hsrc : source P G old with
        | none => rfl
        | some S =>
          cases hdst : (literal P G old new).survivor with
          | none => rfl
          | some D => simp [newPair, hsrc, hdst] at hp
      | mergeOwner =>
        by_cases hm : mapSplit P G old new
        · exact Or.inl ⟨rfl, hm⟩
        apply Or.inr
        refine ⟨H, hdefault, rfl, ?_⟩
        have hp : oldPair P G old new = newPair P G old new := by
          by_cases hp : oldPair P G old new = newPair P G old new
          · exact hp
          · exact False.elim (hx ⟨("merge/content-pair selection", true), by simp [comparisonCases, hp], rfl⟩)
        have hchosen : Rulings11.mergeChosen P.toParams E.rewrite E.merger G old new = some H := by
          unfold Rulings11.answer at ho
          split at ho
          · exact ho
          · contradiction
        exact merge_pair_effect P E hchosen hp hm

/-- Static compatibility causes are separate from operation-selection causes.
Normalization is equality of the configured reader, not equality of outcomes.
It is a higher-order diagnostic, with executable differing-field witnesses. -/
def readerNormalization (G : Graph) : Prop := readParams P G ≠ P

/-- Normalization differences are exactly these three explicit order/policy
fields. Each positive case carries a finite, executable witness. -/
theorem normalization_exact (G : Graph) : readerNormalization P G ↔
    (∃ f g, claimantLe P f g ≠ P.fileLe f g) ∨
    (∃ k, P.aliasKinds k ≠ true) ∨
    (∃ p q, Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe p q ≠ P.pageLe p q) := by
  classical
  constructor
  · intro hn
    by_cases hf : ∃ f g, claimantLe P f g ≠ P.fileLe f g
    · exact Or.inl hf
    right
    by_cases ha : ∃ k, P.aliasKinds k ≠ true
    · exact Or.inl ha
    right
    by_cases hp : ∃ p q, Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe p q ≠ P.pageLe p q
    · exact hp
    have hF : claimantLe P = P.fileLe := by
      funext f g
      by_cases he : claimantLe P f g = P.fileLe f g
      · exact he
      · exact False.elim (hf ⟨f, g, he⟩)
    have hA : (fun _ : Kind => true) = P.aliasKinds := by
      funext k
      by_cases he : P.aliasKinds k = true
      · exact he.symm
      · exact False.elim (ha ⟨k, he⟩)
    have hP : Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe = P.pageLe := by
      funext p q
      by_cases he : Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe p q = P.pageLe p q
      · exact he
      · exact False.elim (hp ⟨p, q, he⟩)
    apply False.elim
    apply hn
    change ({ P.toParams with
      fileLe := claimantLe P
      pageLe := Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe
      aliasKinds := fun _ => true } : Params K) = P
    rw [hP, hF, hA]
  · intro h he
    rcases h with ⟨f, g, h⟩ | ⟨k, h⟩ | ⟨p, q, h⟩
    · exact h (congrArg (fun R : Params K => R.fileLe f g) he)
    · exact h (congrArg (fun R : Params K => R.aliasKinds k) he).symm
    · exact h (congrArg (fun R : Params K => R.pageLe p q) he)

def emptyReal (G : Graph) (s : String) : Bool :=
  (P.real G (P.kindPref, P.key s) && !P.solid G (P.kindPref, P.key s)) ||
  (P.real G (otherKind P.kindPref, P.key s) && !P.solid G (otherKind P.kindPref, P.key s))

/-- Static effects include all declared names, every query, and navigation.
The two reader diagnostics and C4 exhaust all causes of static disagreement. -/
theorem static_comparison_exhaustive (Q : QParams K L) (config : Decided.Config)
    (G : Graph) (s : String) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
    (pages P G = Decided.pages P.toParams G ∧ blocks P G = P.blocks G ∧
      referencesB P G = P.referencesB G ∧ den P Q G = Decided.den P.toParams Q G ∧
      links P config G = Decided.linkB P.toParams config G ∧
      (readParams P G).aliasEdges (activeGraph P G) = P.aliasEdges G ∧
      relationHits P G = (fun t => (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)) ∧
      (fun t => (relationHits P G t).length) = (fun t => ((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).length) ∧
      near P config G = Decided.nearL P.toParams config G ∧
      resolve P G s = P.resolve G s) := by
  classical
  by_cases ha : activeGraph P G = G
  · right
    by_cases hp : readParams P G = P
    · right
      by_cases he : emptyReal P G s = true
      · exact Or.inl he
      right
      obtain ⟨hpages, hblocks, href⟩ := static_overlap P ha hp
      refine ⟨hpages, hblocks, href, by simp [den, ha, hp], ?_, ?_, ?_, ?_, ?_, ?_⟩
      · funext S T; simpa [ha, hp] using links_eq P config G S T
      · simp [ha, hp]
      · funext t; simp only [relationHits, hblocks, href]
      · funext t; simp only [relationHits, hblocks, href]
      · funext d k X; simpa [ha, hp] using near_eq P config G d k X
      · have eqsolid : ∀ k, k = P.kindPref ∨ k = otherKind P.kindPref →
            P.solid G (k, P.key s) = P.real G (k, P.key s) := by
          intro k hk
          cases hr : P.real G (k, P.key s) <;> cases ht : P.solid G (k, P.key s)
          · rfl
          · have h := P.solid_real ht; rw [hr] at h; contradiction
          · rcases hk with rfl | rfl <;> simp [emptyReal, hr, ht] at he
          · rfl
        change Rulings9.resolveKeyR8 (readParams P G) (activeGraph P G) (P.key s) = P.resolve G s
        rw [ha, hp]
        exact (Rulings9.resolve_eq_r8_of_content P.toParams (eqsolid _ (Or.inl rfl)) (eqsolid _ (Or.inr rfl))).symm
    · exact Or.inl hp
  · exact Or.inl ha

/-- The historical model has raw hits only. Applying the current projections
to those hits preserves exhaustive coverage, without inventing old UI policies. -/
theorem static_projection_comparison (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (s : String) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
      ((∀ t, linked P favorites G t =
        ((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).filter (panelSource P favorites G t)) ∧
       (∀ t, referenceCount P favorites G t =
        (((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).filter (panelSource P favorites G t)).length) ∧
       (∀ a q, queryResults P Q R o G a q =
        queryProjection P Q R o G (Decided.den P.toParams Q G a q))) := by
  rcases static_comparison_exhaustive P Q config G s with ha | hp | he | h
  · exact Or.inl ha
  · exact Or.inr (Or.inl hp)
  · exact Or.inr (Or.inr (Or.inl he))
  · right; right; right
    obtain ⟨_, _, _, hden, _, _, hhits, _⟩ := h
    refine ⟨?_, ?_, ?_⟩
    · intro t; simp only [linked, hhits]
    · intro t; simp only [referenceCount, linked, hhits]
    · intro a q; simp only [queryResults, hden]

/-- Comparing the new displayed readings directly with the historical raw
ones also accounts for every difference: either an old reader diagnostic,
an explicitly suppressed input row, or exact equality (including count).
The mask witnesses test source/header/parent rules, never output inequality. -/
theorem static_presented_comparison_exhaustive (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (s : String) (t : Page K) (a : Anchor) (q : Query L) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
      (∃ b ∈ relationHits P G t, panelSource P favorites G t b = false) ∨
      (∃ b, Node.block b ∈ den P Q G a q ∧
        queryRowVisible P Q R o G (den P Q G a q) (.block b) = false) ∨
      (linked P favorites G t = (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t) ∧
       referenceCount P favorites G t = ((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).length ∧
       queryResults P Q R o G a q = Decided.den P.toParams Q G a q) := by
  classical
  rcases static_comparison_exhaustive P Q config G s with ha | hp | he | h
  · exact Or.inl ha
  · exact Or.inr (Or.inl hp)
  · exact Or.inr (Or.inr (Or.inl he))
  · right; right; right
    by_cases hm : ∃ b ∈ relationHits P G t, panelSource P favorites G t b = false
    · exact Or.inl hm
    right
    by_cases hn : ∃ b, Node.block b ∈ den P Q G a q ∧
        queryRowVisible P Q R o G (den P Q G a q) (.block b) = false
    · exact Or.inl hn
    right
    obtain ⟨_, _, _, hden, _, _, hhits, _⟩ := h
    have hl : linked P favorites G t = relationHits P G t := by
      apply List.filter_eq_self.mpr
      intro b hb
      cases hv : panelSource P favorites G t b with
      | true => rfl
      | false => exact False.elim (hm ⟨b, hb, hv⟩)
    have hq : queryResults P Q R o G a q = den P Q G a q := by
      apply List.filter_eq_self.mpr
      intro n hn'
      cases n with
      | page _ => rfl
      | block b =>
        cases hv : queryRowVisible P Q R o G (den P Q G a q) (.block b) with
        | true => rfl
        | false => exact False.elim (hn ⟨b, hn', hv⟩)
    exact ⟨hl.trans (congrFun hhits t), by rw [referenceCount, hl, hhits], by rw [hq, hden]⟩

end Consolidated13
end CM
