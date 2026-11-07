/-
The graph's page and node universe (SPEC §2.3, §3.2): real pagesLegacyA, reference-only pagesLegacyA, `pages G`,
`nodes G`, and the T1 extension that both depend only on the set of files. (Round 1 fix, verifier
finding 1.)
-/
import Locality

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-- Is key `k` claimed by some file, of either kind? -/
def claimedKey (G : Graph) (k : K) : Bool := P.real G (.journal, k) || P.real G (.page, k)

/-- The keys referenced by owner blocks: the keys of their namespace-closed occurrence spellings
(SPEC §3.2). -/
def refKeys (G : Graph) : List K := (P.blocks G).flatMap (fun b => (P.refNames G b).map P.key)

/-- SPEC §3.2: every referenced key that no file claims is a reference-only page, of kind
`refKind k` (O3). Listed with repetitions; membership is what matters. -/
def refOnlyPagesLegacyA (G : Graph) : List (Page K) :=
  ((P.refKeys G).filter (fun k => !P.claimedKey G k)).map (fun k => (P.refKind k, k))

/-- SPEC §2.3: `pages G` = real pages ∪ reference-only pages. -/
def pagesLegacyA (G : Graph) : List (Page K) := P.realPages G ++ P.refOnlyPagesLegacyA G

end Params

/-- A node is a page or a block (SPEC §2.3). -/
inductive Node (K : Type) | page (p : Page K) | block (b : BlockId)
  deriving DecidableEq, Repr

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-- `nodes G = pages G ∪ blocks G`. -/
def nodesLegacyA (G : Graph) : List (Node K) := (P.pagesLegacyA G).map .page ++ (P.blocks G).map .block

/-! ## Membership -/

theorem mem_realPages {G : Graph} {p : Page K} : p ∈ P.realPages G ↔ P.real G p = true := by
  rw [P.real_iff]; simp [realPages]

theorem claimedKey_iff {G : Graph} {k : K} : P.claimedKey G k = true ↔ ∃ f ∈ G, (P.claim f).2 = k := by
  simp only [claimedKey, Bool.or_eq_true, P.real_iff]
  constructor
  · rintro (⟨f, hf, e⟩ | ⟨f, hf, e⟩) <;> exact ⟨f, hf, by rw [e]⟩
  · rintro ⟨f, hf, e⟩
    have hc : P.claim f = (kindOf f, k) := by rw [← e]; rfl
    cases hk : kindOf f
    · exact Or.inl ⟨f, hf, by rw [hc, hk]⟩
    · exact Or.inr ⟨f, hf, by rw [hc, hk]⟩

theorem mem_refKeys {G : Graph} {k : K} :
    k ∈ P.refKeys G ↔ ∃ b ∈ P.blocks G, ∃ s ∈ P.refNames G b, P.key s = k := by
  simp [refKeys]

theorem mem_refOnlyPages {G : Graph} {p : Page K} :
    p ∈ P.refOnlyPagesLegacyA G ↔ p = (P.refKind p.2, p.2) ∧ p.2 ∈ P.refKeys G ∧ P.claimedKey G p.2 = false := by
  obtain ⟨κ, k⟩ := p
  simp only [refOnlyPagesLegacyA, List.mem_map, List.mem_filter, Bool.not_eq_true', Prod.mk.injEq]
  constructor
  · rintro ⟨k', ⟨h1, h2⟩, e1, e2⟩; subst e2; exact ⟨⟨e1.symm, trivial⟩, h1, h2⟩
  · rintro ⟨⟨e1, _⟩, h1, h2⟩; exact ⟨k, ⟨h1, h2⟩, e1.symm, rfl⟩

/-- Reference-only pages are not real, of either kind (the union is disjoint by key). -/
theorem refOnly_not_real {G : Graph} {p : Page K} (h : p ∈ P.refOnlyPagesLegacyA G) (κ : Kind) :
    P.real G (κ, p.2) = false := by
  have hc := ((P.mem_refOnlyPages).mp h).2.2
  cases hr : P.real G (κ, p.2) with
  | false => rfl
  | true =>
    obtain ⟨f, hf, e⟩ := (P.real_iff).mp hr
    have : P.claimedKey G p.2 = true := (P.claimedKey_iff).mpr ⟨f, hf, by rw [e]⟩
    rw [hc] at this; cases this

/-! ## T1 extension: the page and node universe depends only on the set of files -/

variable {P}

theorem blocks_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (b : BlockId) :
    b ∈ P.blocks G₁ ↔ b ∈ P.blocks G₂ := by
  rw [mem_blocks, mem_blocks]
  constructor
  · rintro ⟨f, hf, ho, r⟩; exact ⟨f, (h f).mp hf, by rw [← isOwner_congr hP h]; exact ho, r⟩
  · rintro ⟨f, hf, ho, r⟩; exact ⟨f, (h f).mpr hf, by rw [isOwner_congr hP h]; exact ho, r⟩

theorem refKeys_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂) (k : K) :
    k ∈ P.refKeys G₁ ↔ k ∈ P.refKeys G₂ := by
  rw [P.mem_refKeys, P.mem_refKeys]
  have hr : ∀ b, P.refNames G₁ b = P.refNames G₂ b := fun b => by
    unfold refNames; rw [occAt_sameFiles hw h]
  constructor
  · rintro ⟨b, hb, r⟩; exact ⟨b, (blocks_sameFiles hP h b).mp hb, by rw [← hr]; exact r⟩
  · rintro ⟨b, hb, r⟩; exact ⟨b, (blocks_sameFiles hP h b).mpr hb, by rw [hr]; exact r⟩

/-- **T1 (pages).** `pages G` depends only on the set of files. -/
theorem pages_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂) (p : Page K) :
    p ∈ P.pagesLegacyA G₁ ↔ p ∈ P.pagesLegacyA G₂ := by
  simp only [pagesLegacyA, List.mem_append, P.mem_realPages, P.mem_refOnlyPages, real_congr h,
    refKeys_sameFiles hP hw h]
  have : P.claimedKey G₁ p.2 = P.claimedKey G₂ p.2 := by simp only [claimedKey, real_congr h]
  rw [this]

/-- **T1 (nodes).** So does `nodes G`. -/
theorem nodes_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂) (n : Node K) :
    n ∈ P.nodesLegacyA G₁ ↔ n ∈ P.nodesLegacyA G₂ := by
  cases n with
  | page p => simp only [nodesLegacyA, List.mem_append, List.mem_map, Node.page.injEq, reduceCtorEq,
      and_false, exists_false, or_false]; exact ⟨fun ⟨q, hq, e⟩ => ⟨q, (pages_sameFiles hP hw h q).mp hq, e⟩,
        fun ⟨q, hq, e⟩ => ⟨q, (pages_sameFiles hP hw h q).mpr hq, e⟩⟩
  | block b => simp only [nodesLegacyA, List.mem_append, List.mem_map, Node.block.injEq, reduceCtorEq,
      and_false, exists_false, false_or]; exact ⟨fun ⟨q, hq, e⟩ => ⟨q, (blocks_sameFiles hP h q).mp hq, e⟩,
        fun ⟨q, hq, e⟩ => ⟨q, (blocks_sameFiles hP h q).mpr hq, e⟩⟩

end Params
end CM
