/- Round 7 (D-20261102-CM4, CM-CASE): the rename guard counts only pages OTHER
than the renamed ones.

`Decided.renameBaseAllowed` now uses `Decided.renameCoreAllowed` (rule 6.4's
alias-class clause refuses only a real page with a key different from the
source's) and exempts, in a same-key rename, every moved spelling whose key the
substitution leaves unchanged from the real-target clause (`Decided.keepsKey`).

This module keeps the round-6 guard verbatim as `renameBaseAllowedR6` and proves
1. the fix changes nothing when `key new ≠ key old` (`base_eq_r6`, `rename_eq_r6`):
   every other refusal is kept, for every graph;
2. the round-6 guard refused EVERY same-key rename (`r6_refuses_same_key`);
3. under explicit key laws, a same-key rename is refused exactly when the source
   is not a real page or a moved name is a duplicate claim (`same_key_allowed_iff`);
4. an accepted same-key rename changes no page, block-to-page reference or
   page's blocks beyond their positions' new paths (T3 at σ = id,
   `same_key_transport`). -/
import QueryWitness6
set_option linter.unusedSectionVars false

namespace CM
namespace CaseRename7
open Params Decided

variable {K : Type} [DecidableEq K] (P : Params K)

/-- The round-6 base guard, verbatim, kept for comparison. -/
def renameBaseAllowedR6 (G : Graph) (old new : String) : Bool :=
  P.renameAllowedLegacy G old new &&
  (movedNames P G old).all (fun s =>
    !P.conflictedB G (P.key s) &&
    !P.conflictedB G (P.key (nsSubst P.key P.urlHasOrigin old new s))) &&
  !P.conflictedB G (P.key new) &&
  (P.movedFiles G old).all (fun f => (P.movedFiles G old).all (fun g =>
    decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) =
      P.key (nsSubst P.key P.urlHasOrigin old new (P.name g)) → (P.claim f).2 = (P.claim g).2))) &&
  G.all (fun g => decide (g ∈ P.movedFiles G old) ||
    (movedPageNames P G old).all (fun s =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new s))))

def renameAllowedR6 (G : Graph) (old new : String) : Bool :=
  renameBaseAllowedR6 P G old new && (journalRewriteRisks P G old new).isEmpty

/-- The round-6 executable rename, for comparison. -/
def renameR6 (R : Decided.Rewriter) (G : Graph) (old new : String) : Option Graph :=
  if renameAllowedR6 P G old new then some (P.rename R.base G old new) else none

/-! ## 1. Nothing changes for a different key -/

theorem classClash_iff {G : Graph} {old new : String} :
    classClash P G old new = true ↔
      P.key new ≠ P.key old ∧ (∃ f ∈ G, (P.claim f).2 = P.key new) ∧
        cls (P.aliasEdges G) (P.key old) (P.key new) := by
  simp only [classClash, Bool.and_eq_true, decide_eq_true_eq, P.claimedKey_iff, sameCls_iff]
  exact ⟨fun ⟨⟨a, b⟩, c⟩ => ⟨a, b, c⟩, fun ⟨a, b, c⟩ => ⟨⟨a, b⟩, c⟩⟩

theorem core_eq_legacy {G : Graph} {old new : String} (h : P.key new ≠ P.key old) :
    renameCoreAllowed P G old new = P.renameAllowedLegacy G old new := by
  unfold renameCoreAllowed classClash Params.renameAllowedLegacy
  simp only [ne_eq, h, not_false_eq_true, decide_true, Bool.true_and]

theorem keepsKey_false {old new s : String} (h : P.key new ≠ P.key old) :
    keepsKey P old new s = false := by
  simp [keepsKey, h]

/-- **Every refusal of round 6 is kept** whenever the new key differs. -/
theorem base_eq_r6 {G : Graph} {old new : String} (h : P.key new ≠ P.key old) :
    renameBaseAllowed P G old new = renameBaseAllowedR6 P G old new := by
  unfold renameBaseAllowed renameBaseAllowedR6
  rw [core_eq_legacy P h]
  simp only [keepsKey_false P h, Bool.or_false]

/-- Round 9: restated (ruling-driven, CM-OWN-CHILD). For `key new ≠ key old`
the decided guard is the round-6 guard plus the round-9 own-child refusal;
the round-8 equation is `allowed_r8_eq_r6`. -/
theorem allowed_eq_r6 {G : Graph} {old new : String} (h : P.key new ≠ P.key old) :
    renameAllowed P G old new = (renameAllowedR6 P G old new && !ownChildUsed P G old new) := by
  unfold renameAllowed renameAllowedR6
  rw [base_eq_r6 P h]

/-- The round-8 statement of `allowed_eq_r6`, for the historical round-8 guard. -/
theorem allowed_r8_eq_r6 {G : Graph} {old new : String} (h : P.key new ≠ P.key old) :
    renameAllowedR8 P G old new = renameAllowedR6 P G old new := by
  unfold renameAllowedR8 renameAllowedR6
  rw [base_eq_r6 P h]

/-- Round 9: restated with the hypothesis that the own-child clause does not
fire (ruling-driven); every round-6 refusal is still kept (`allowed_eq_r6`). -/
theorem rename_eq_r6 (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : P.key new ≠ P.key old) (hc : ownChildUsed P G old new = false) :
    Decided.rename P R G old new = renameR6 P R G old new := by
  unfold Decided.rename renameR6
  rw [allowed_eq_r6 P h, hc, Bool.not_false, Bool.and_true]

/-! ## 2. The round-6 defect -/

theorem sameCls_self {α : Type} [DecidableEq α] (E : List (α × α)) (x : α) : sameCls E x x = true :=
  (sameCls_iff E x x).mpr (cls_self E x)

/-- The round-6 legacy guard refused every rename whose new key equals the old. -/
theorem legacy_refuses_same_key {G : Graph} {old new : String} (h : P.key new = P.key old) :
    P.renameAllowedLegacy G old new = false := by
  unfold Params.renameAllowedLegacy
  cases hr : P.real G (.page, P.key old) with
  | false => simp
  | true =>
    have hc : P.claimedKey G (P.key new) = true := by
      unfold claimedKey; rw [h, hr]; simp
    rw [hc, h, sameCls_self]
    simp

/-- **CM-CASE, the defect.** Round 6 refused every same-key rename (`Foo` → `foo`). -/
theorem r6_refuses_same_key {G : Graph} {old new : String} (h : P.key new = P.key old) :
    renameAllowedR6 P G old new = false := by
  unfold renameAllowedR6 renameBaseAllowedR6
  rw [legacy_refuses_same_key P h]
  simp

theorem classClash_same_key {G : Graph} {old new : String} (h : P.key new = P.key old) :
    classClash P G old new = false := by
  simp [classClash, h]

/-! ## 3. Exactly which same-key renames are refused -/

/-- Key laws for a same-key rename: namespace closure keys are unchanged by the
substitution, and whether a spelling lies in `old`'s subtree depends only on its
key. Both hold for Tine's case/Unicode folding key; they are hypotheses here, as
K1 alone implies neither. -/
structure SameKeyLaws (old new : String) : Prop where
  same : P.key new = P.key old
  closure : ∀ s, (P.occClosure (nsSubst P.key P.urlHasOrigin old new s)).map P.key = (P.occClosure s).map P.key
  subtree : ∀ s t, P.key s = P.key t →
    (nsMatch P.key P.urlHasOrigin old s).isSome = (nsMatch P.key P.urlHasOrigin old t).isSome

theorem nsClosure_head (s : String) : ∃ l, nsClosure s = s :: l := ⟨_, rfl⟩

/-- Round 10: the occurrence closure also starts with the spelling itself. -/
theorem occClosure_head (s : String) : ∃ l, P.occClosure s = s :: l := by
  unfold occClosure; split
  · exact ⟨_, rfl⟩
  · exact ⟨[], rfl⟩

variable {P} in
theorem SameKeyLaws.subst_key {old new : String} (L : SameKeyLaws P old new) (s : String) :
    P.key (nsSubst P.key P.urlHasOrigin old new s) = P.key s := by
  have h := L.closure s
  obtain ⟨l, hl⟩ := occClosure_head P (nsSubst P.key P.urlHasOrigin old new s)
  obtain ⟨l', hl'⟩ := occClosure_head P s
  rw [hl, hl', List.map_cons, List.map_cons] at h
  exact (List.cons.inj h).1

variable {P} in
theorem SameKeyLaws.rwOcc_closure {old new : String} (L : SameKeyLaws P old new)
    (o : OccKind × String) :
    (P.occClosure (rwOcc covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key = (P.occClosure o.2).map P.key := by
  unfold rwOcc
  split
  · exact L.closure o.2
  · rfl

theorem old_mem_movedNames {G : Graph} {old : String} : old ∈ movedNames P G old :=
  List.mem_cons_self ..

theorem name_mem_movedNames {G : Graph} {old : String} {f : File} (hf : f ∈ G)
    (hm : (nsMatch P.key P.urlHasOrigin old (P.name f)).isSome = true) : P.name f ∈ movedNames P G old := by
  unfold movedNames usedNames
  apply List.mem_cons_of_mem
  simp only [List.mem_filter, List.mem_append, List.mem_map]
  exact ⟨Or.inl (Or.inl (Or.inl ⟨f, hf, rfl⟩)), hm⟩

theorem no_journal_risk {G : Graph} {old new : String} (L : SameKeyLaws P old new) :
    ¬ JournalRisk P G old new := by
  rintro ⟨b, _, o, _, k, hk, _, hn⟩
  rw [L.rwOcc_closure] at hn
  exact hn hk

/-- **CM-CASE, the repair (exact).** Under the same-key laws, the decided guard
accepts a same-key rename exactly when the source is a real page and no moved
name (the source, its namespace descendants, used names below it) is claimed by
two distinct files. In particular no page counts as a collision with itself. -/
theorem same_key_allowed_iff {G : Graph} {old new : String} (L : SameKeyLaws P old new) :
    renameAllowed P G old new = true ↔
      P.real G (.page, P.key old) = true ∧
        ∀ s ∈ movedNames P G old, ¬ P.Conflicted G (P.key s) := by
  have hrisk : (journalRewriteRisks P G old new).isEmpty = true := by
    cases h : (journalRewriteRisks P G old new).isEmpty
    · exact absurd ((journalRewriteRisks_nonempty P).mp h) (no_journal_risk P L)
    · rfl
  have hown : ownChildUsed P G old new = false := by simp [ownChildUsed, L.same]
  unfold renameAllowed renameBaseAllowed renameCoreAllowed
  rw [hrisk, classClash_same_key P L.same, hown]
  simp only [Bool.and_true, Bool.not_false, Bool.and_eq_true, List.all_eq_true,
    Bool.not_eq_true', Bool.or_eq_true, decide_eq_true_eq]
  constructor
  · rintro ⟨⟨⟨⟨⟨hr, _⟩, hB⟩, _⟩, _⟩, _⟩
    refine ⟨hr, fun s hs hc => ?_⟩
    have := (hB s hs).1
    rw [P.conflictedB_iff.mpr hc] at this
    cases this
  · rintro ⟨hr, hc⟩
    have hcB : ∀ s ∈ movedNames P G old, P.conflictedB G (P.key s) = false := fun s hs => by
      cases h : P.conflictedB G (P.key s)
      · rfl
      · exact absurd (P.conflictedB_iff.mp h) (hc s hs)
    refine ⟨⟨⟨⟨⟨hr, ?_⟩, ?_⟩, ?_⟩, ?_⟩, ?_⟩
    · -- subtree clause: a file outside the subtree claiming a moved key is a duplicate claim
      intro g hg
      by_cases hin : kindOf g = .page ∧ (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome = true
      · exact Or.inl (by simp [hin.1, hin.2])
      · refine Or.inr (fun f hf => ?_)
        simp only [movedFiles, List.mem_filter, Bool.and_eq_true, decide_eq_true_eq] at hf
        obtain ⟨hfG, hfk, hfm⟩ := hf
        intro he
        rw [L.subst_key] at he
        change P.key (P.name g) = P.key (P.name f) at he
        by_cases hgk : kindOf g = .page
        · apply hin ⟨hgk, ?_⟩
          rw [L.subtree _ _ he]; exact hfm
        · have hne : g ≠ f := fun e => hgk (e ▸ hfk)
          apply hc (P.name f) (name_mem_movedNames P hfG hfm)
          exact ⟨g, hg, f, hfG, hne, he, rfl⟩
    · intro s hs
      rw [L.subst_key, hcB s hs]
      simp
    · have := hcB old (old_mem_movedNames P)
      rw [L.same, this]
    · intro f _ g _ he
      rw [L.subst_key, L.subst_key] at he
      exact he
    · intro g _
      refine Or.inr (fun s _ => Or.inr ?_)
      simp [keepsKey, L.same, L.subst_key]

/-! ## 4. What an accepted same-key rename changes: nothing but spellings -/

/-- The key-level hypotheses of T3 hold with σ = id under the same-key laws,
given the file-name extractor's agreement on namespace parents. -/
theorem same_key_keyHyps {R : CM.Rewriter} {G : Graph} {old new : String}
    (L : SameKeyLaws P old new)
    (hnames : ∀ f ∈ G, (P.fileNsNames (R.apply (nsSubst P.key P.urlHasOrigin old new) f)).map P.key =
      (P.fileNsNames f).map P.key) :
    P.KeyHyps R (nsSubst P.key P.urlHasOrigin old new) id G where
  alias := fun _ _ s _ => L.subst_key s
  ns := fun _ _ _ _ o _ _ => by
    rw [L.closure o.2]
    exact ⟨fun k hk => by simpa using hk, fun k hk => by simpa using hk⟩
  names := fun f hf => by
    rw [hnames f hf]
    exact ⟨fun k hk => by simpa using hk, fun k hk => by simpa using hk⟩
  inj := fun _ _ _ _ e => e
  refKind := fun _ _ => rfl

theorem σp_id (p : Page K) : σp id p = p := rfl

/-- **Same-key transport.** For a same-key rename satisfying the rewriter's
extractor laws, the page universe is unchanged, every block keeps exactly the
pages it references (at its image position), and every page keeps its blocks.
No exception-freeness is needed: σ = id moves no key. -/
theorem same_key_transport {R : CM.Rewriter} {G : Graph} {old new : String}
    (L : SameKeyLaws P old new)
    (hw : WF G) (hw' : WF (renameBy R (nsSubst P.key P.urlHasOrigin old new) G))
    (hL : P.RwLaws R (nsSubst P.key P.urlHasOrigin old new) id G)
    (hnames : ∀ f ∈ G, (P.fileNsNames (R.apply (nsSubst P.key P.urlHasOrigin old new) f)).map P.key =
      (P.fileNsNames f).map P.key) :
    (∀ (r : Bool) (p : Page K),
      p ∈ P.pagesR r (renameBy R (nsSubst P.key P.urlHasOrigin old new) G) ↔ p ∈ P.pagesR r G) ∧
    (∀ b ∈ P.blocks G, ∀ t : Page K, P.InPlay G t.2 →
      (P.References G b t ↔
        P.References (renameBy R (nsSubst P.key P.urlHasOrigin old new) G)
          (blockMap R (nsSubst P.key P.urlHasOrigin old new) G b) t)) ∧
    (∀ p : Page K, P.InPlay G p.2 →
      P.pageBlocks (renameBy R (nsSubst P.key P.urlHasOrigin old new) G) p =
        (P.pageBlocks G p).map (blockMap R (nsSubst P.key P.urlHasOrigin old new) G)) := by
  have H : P.RenameHyps R (nsSubst P.key P.urlHasOrigin old new) id G :=
    ⟨hw, hw', hL, same_key_keyHyps P L hnames⟩
  have hx : P.GraphExceptionFree R id G := fun _ _ _ _ _ _ _ => rfl
  refine ⟨fun r p => ⟨fun hp => ?_, fun hp => ?_⟩, fun b hb t ht => ?_, fun p hp => ?_⟩
  · obtain ⟨q, hq, e⟩ := pagesR_rename_onto H hx hp
    rw [← e]; exact hq
  · exact pagesR_rename_fwd H hx hp
  · exact references_rename H hb (fun _ _ _ _ _ => rfl) ht
  · exact pageBlocks_rename H hp

end CaseRename7
end CM
