/- Exact transport under non-injective renames. Real claimant keys stay
injective, but reference-only keys and alias-only keys may merge. No namespace
compatibility or reference-key injectivity is assumed by the exact formula. -/
import Isolation -- Build ordering: at most two concurrent module compilers.
set_option linter.unusedSectionVars false
namespace CM
namespace Params
variable {K : Type} [DecidableEq K] (P : Params K)
structure MergeHyps (R : CM.Rewriter) (m : String → String) (σ : K → K) (G : Graph) : Prop where
  wf : WF G
  wf' : WF (renameBy R m G)
  laws : P.RwLaws R m σ G
  alias : ∀ f ∈ G, ∀ s ∈ P.aliasDecl f.format f.text, P.key (m s) = σ (P.key s)
  realInj : ∀ f ∈ G, ∀ g ∈ G, σ (P.claim f).2 = σ (P.claim g).2 → (P.claim f).2 = (P.claim g).2

variable {P} {R : CM.Rewriter} {m : String → String} {σ : K → K} {G : Graph}

theorem claimants_merge (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) :
    P.claimants (renameBy R m G) (σp σ (P.claim f)) = (P.claimants G (P.claim f)).map (R.apply m) := by
  unfold claimants renameBy
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro g hg
  simp only [Function.comp, claim_apply H.laws hg]
  apply decide_eq_decide.mpr
  constructor
  · intro he
    have hk := congrArg (fun p : Page K => p.1) he
    have hv := congrArg (fun p : Page K => p.2) he
    exact Prod.ext hk (H.realInj g hg f hf hv)
  · intro he; rw [he]

theorem owner_merge (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) :
    P.owner (renameBy R m G) (σp σ (P.claim f)) = (P.owner G (P.claim f)).map (R.apply m) := by
  unfold owner
  rw [claimants_merge H hf]
  apply minBy_map
  intro x hx y hy
  rw [P.mem_claimants] at hx hy
  exact H.laws.order x hx.1 y hy.1 (hx.2.trans hy.2.symm)

theorem isOwner_merge (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) :
    P.isOwner (renameBy R m G) (R.apply m f) = P.isOwner G f := by
  unfold isOwner
  rw [claim_apply H.laws hf, owner_merge H hf]
  apply decide_eq_decide.mpr
  constructor
  · intro e
    obtain ⟨g, hg, eg⟩ := Option.map_eq_some_iff.mp e
    have hgG := (P.owner_spec hg).1
    rw [hg, apply_inj H.wf' hgG hf eg]
  · intro e; rw [e]; rfl

theorem declaringOwners_merge (H : P.MergeHyps R m σ G) :
    P.declaringOwners (renameBy R m G) = (P.declaringOwners G).map (R.apply m) := by
  unfold declaringOwners
  unfold renameBy
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro f hf
  have hk : kindOf (R.apply m f) = kindOf f := by unfold kindOf; rw [H.laws.dir f hf]
  simp only [Function.comp]
  rw [← renameBy, isOwner_merge H hf, hk]

theorem aliasEdges_merge (H : P.MergeHyps R m σ G) (e : K × K) :
    e ∈ P.aliasEdges (renameBy R m G) ↔ e ∈ Reach.mapE σ (P.aliasEdges G) := by
  obtain ⟨a, b⟩ := e
  rw [P.mem_aliasEdges, declaringOwners_merge H]
  unfold Reach.mapE
  rw [List.mem_map]
  constructor
  · rintro ⟨f', hf', ha, s', hs', hb⟩
    obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hf'
    have hfG := ((P.mem_declaringOwners).mp hf).1
    rw [H.laws.alias f hfG] at hs'
    obtain ⟨s, hs, rfl⟩ := List.mem_map.mp hs'
    refine ⟨((P.claim f).2, P.key s), (P.mem_aliasEdges).mpr ⟨f, hf, rfl, s, hs, rfl⟩, ?_⟩
    rw [claim_apply H.laws hfG] at ha
    rw [← ha, ← hb, H.alias f hfG s hs]; rfl
  · rintro ⟨⟨a0, b0⟩, he, ee⟩
    obtain ⟨f, hf, ha, s, hs, hb⟩ := (P.mem_aliasEdges).mp he
    have hfG := ((P.mem_declaringOwners).mp hf).1
    simp only [Prod.mk.injEq] at ee
    refine ⟨R.apply m f, List.mem_map_of_mem hf, ?_, m s, ?_, ?_⟩
    · rw [claim_apply H.laws hfG]; simp only [σp]; rw [ha]; exact ee.1
    · rw [H.laws.alias f hfG]; exact List.mem_map_of_mem hs
    · rw [H.alias f hfG s hs, hb]; exact ee.2

theorem occAt_merge (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) (i : Nat) :
    P.occAt (renameBy R m G) ((R.apply m f).path, i) =
      (P.occAt G (f.path, i)).map (rwOcc R.covers m) := by
  rw [occAt_some H.wf hf, occAt_some H.wf' (List.mem_map_of_mem hf)]
  have h := congrArg (fun l => l[i]?) (H.laws.occ f hf)
  simp only [List.getElem?_map] at h
  rw [← h]
  cases (P.outline f.format f.text)[i]? <;> rfl


theorem blocks_merge (H : P.MergeHyps R m σ G) (b' : BlockId) :
    b' ∈ P.blocks (renameBy R m G) ↔ ∃ b ∈ P.blocks G, blockMap R m G b = b' := by
  rw [P.mem_blocks]
  constructor
  · rintro ⟨f', hf', ho, ep, hi⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    refine ⟨(f.path, b'.2), (P.mem_blocks).mpr ⟨f, hf, ?_, rfl, ?_⟩, ?_⟩
    · rw [← isOwner_merge H hf]; exact ho
    · rw [← outline_length_rename H.laws hf]; exact hi
    · rw [blockMap_eq H.wf hf, ep]
  · rintro ⟨b, hb, rfl⟩
    obtain ⟨f, hf, ho, ep, hi⟩ := (P.mem_blocks).mp hb
    obtain ⟨π, i⟩ := b
    simp only at ep hi; subst ep
    rw [blockMap_eq H.wf hf]
    exact ⟨R.apply m f, List.mem_map_of_mem hf, by rw [isOwner_merge H hf]; exact ho, rfl,
      by rw [outline_length_rename H.laws hf]; exact hi⟩

theorem claimedKey_merge (H : P.MergeHyps R m σ G) (k' : K) :
    P.claimedKey (renameBy R m G) k' = true ↔ ∃ k, P.claimedKey G k = true ∧ σ k = k' := by
  rw [P.claimedKey_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    rw [claim_apply H.laws hf] at e
    exact ⟨(P.claim f).2, (P.claimedKey_iff).mpr ⟨f, hf, rfl⟩, e⟩
  · rintro ⟨k, hk, rfl⟩
    obtain ⟨f, hf, e⟩ := (P.claimedKey_iff).mp hk
    exact ⟨R.apply m f, List.mem_map_of_mem hf, by rw [claim_apply H.laws hf]; simp [σp, e]⟩

theorem real_merge (H : P.MergeHyps R m σ G) (q : Page K) :
    P.real (renameBy R m G) q = true ↔ ∃ p, P.real G p = true ∧ σp σ p = q := by
  rw [P.real_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    rw [claim_apply H.laws hf] at e
    exact ⟨P.claim f, (P.real_iff).mpr ⟨f, hf, rfl⟩, e⟩
  · rintro ⟨p, hp, rfl⟩
    obtain ⟨f, hf, rfl⟩ := (P.real_iff).mp hp
    exact ⟨R.apply m f, List.mem_map_of_mem hf, claim_apply H.laws hf⟩

/-- Exact post-rename references, expressed in OLD raw occurrences and OLD edges.
Merges are represented by mapping the edges before taking connected components;
new namespace prefixes and permanent exceptions are represented by rwOcc/nsClosure. -/
theorem references_merge_exact (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G)
    (i : Nat) (t : Page K) :
    P.References (renameBy R m G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i), ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
        cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := by
  rw [P.references_iff_keys]
  unfold refNames
  rw [occAt_merge H hf]
  simp only [List.mem_flatMap, List.mem_map]
  constructor
  · rintro ⟨u, ⟨o', ⟨o, ho, rfl⟩, hu⟩, hc⟩
    exact ⟨o, ho, u, hu, (Reach.congr (aliasEdges_merge H)).mp hc⟩
  · rintro ⟨o, ho, u, hu, hc⟩
    exact ⟨u, ⟨_, ⟨o, ho, rfl⟩, hu⟩, (Reach.congr (aliasEdges_merge H)).mpr hc⟩

/-- Adding the explicit forward namespace-closure and exception conditions yields
reference preservation in the forward direction even when keys merge. -/
theorem references_merge_forward (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G)
    (i : Nat) {t : Page K}
    (hns : ∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2,
      ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, P.key v = σ (P.key u))
    (hr : P.References G (f.path, i) t) :
    P.References (renameBy R m G) ((R.apply m f).path, i) (σp σ t) := by
  rw [references_merge_exact H hf]
  obtain ⟨u, hu, hc⟩ := P.references_iff_keys.mp hr
  obtain ⟨o, ho, hu⟩ := P.mem_refNames.mp hu
  obtain ⟨v, hv, he⟩ := hns o ho u hu
  exact ⟨o, ho, v, hv, he ▸ Reach.map_fwd σ hc⟩

/-- Real pages keep exactly their own blocks under transport, even when they
absorb a reference-only page. A reference-only target may GAIN those blocks. -/
theorem pageBlocks_merge_real (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) :
    P.pageBlocks (renameBy R m G) (σp σ (P.claim f)) =
      (P.pageBlocks G (P.claim f)).map (blockMap R m G) := by
  unfold pageBlocks
  rw [owner_merge H hf]
  cases ho : P.owner G (P.claim f) with
  | none => rfl
  | some g =>
    have hg := (P.owner_spec ho).1
    simp only [Option.map_some, List.map_map]
    rw [outline_length_rename H.laws hg]
    apply List.map_congr_left
    intro i _
    simp only [Function.comp]
    rw [blockMap_eq H.wf hg]

/-- Fresh transport is a special case, but the exact formula also covers the
accepted reference-only merge witnesses, where RenameHyps.keys.inj is false. -/
theorem renameHyps_toMerge (H : P.RenameHyps R m σ G) : P.MergeHyps R m σ G where
  wf := H.wf
  wf' := H.wf'
  laws := H.laws
  alias := H.keys.alias
  realInj := fun _ hf _ hg he => inPlay_inj H.keys (P.inPlay_claim hf) (P.inPlay_claim hg) he

/-- Raw reference keys after rewriting, computed entirely from old owner blocks.
This includes NEW namespace parents and UNCHANGED exception spellings. -/
def rewrittenRefKeys (P : Params K) (R : CM.Rewriter) (m : String → String) (G : Graph) : List K :=
  (P.blocks G).flatMap (fun b => (P.occAt G b).flatMap (fun o =>
    (P.occClosure (rwOcc R.covers m o).2).map P.key))

theorem refKeys_merge_exact (H : P.MergeHyps R m σ G) (k : K) :
    k ∈ P.refKeys (renameBy R m G) ↔ k ∈ rewrittenRefKeys P R m G := by
  rw [P.mem_refKeys]
  simp only [rewrittenRefKeys, List.mem_flatMap, List.mem_map]
  constructor
  · rintro ⟨b', hb', u, hu, he⟩
    obtain ⟨b, hb, rfl⟩ := (blocks_merge H _).mp hb'
    obtain ⟨f, hf, ho, hp, hi⟩ := P.mem_blocks.mp hb
    have hbid : b = (f.path, b.2) := Prod.ext hp.symm rfl
    rw [hbid, blockMap_eq H.wf hf] at hu
    obtain ⟨o', ho', hu⟩ := P.mem_refNames.mp hu
    rw [occAt_merge H hf] at ho'
    obtain ⟨o, hoc, rfl⟩ := List.mem_map.mp ho'
    refine ⟨b, hb, o, ?_, u, hu, he⟩
    rw [hbid]; exact hoc
  · rintro ⟨b, hb, o, ho, u, hu, he⟩
    obtain ⟨f, hf, hio, hp, hi⟩ := P.mem_blocks.mp hb
    have hbid : b = (f.path, b.2) := Prod.ext hp.symm rfl
    refine ⟨blockMap R m G b, (blocks_merge H _).mpr ⟨b, hb, rfl⟩, u, ?_, he⟩
    rw [hbid, blockMap_eq H.wf hf]
    apply P.mem_refNames.mpr
    refine ⟨rwOcc R.covers m o, ?_, hu⟩
    rw [occAt_merge H hf]
    exact List.mem_map_of_mem (by rw [← hbid]; exact ho)

/-- Recompute file-name parents from each rewritten old file. No commuting
namespace or blankness assumption is needed for this exact transformation. -/
def rewrittenNameKeys (P : Params K) (R : CM.Rewriter) (m : String → String) (G : Graph) : List K :=
  G.flatMap (fun f => (P.fileNsNames (R.apply m f)).map P.key)

def rewrittenPageSeedKeys (P : Params K) (R : CM.Rewriter) (m : String → String) (G : Graph) : List K :=
  rewrittenRefKeys P R m G ++ rewrittenNameKeys P R m G

theorem nameKeys_merge_exact (k : K) :
    k ∈ P.nameKeys (renameBy R m G) ↔ k ∈ rewrittenNameKeys P R m G := by
  simp only [nameKeys, renameBy, List.flatMap_map, rewrittenNameKeys]

theorem pageSeedKeys_merge_exact (H : P.MergeHyps R m σ G) (k : K) :
    k ∈ P.pageSeedKeys (renameBy R m G) ↔ k ∈ rewrittenPageSeedKeys P R m G := by
  simp only [pageSeedKeys, rewrittenPageSeedKeys, List.mem_append,
    refKeys_merge_exact H, nameKeys_merge_exact]

/-- Exact page universe under merges; no surjection, freshness, namespace
compatibility or refKind-invariance is presumed. All right-hand data is old data. -/
theorem pages_merge_exact (H : P.MergeHyps R m σ G) (p : Page K) :
    p ∈ Decided.pages P (renameBy R m G) ↔
      (∃ q, P.real G q = true ∧ σp σ q = p) ∨
      (p = (P.refKind p.2, p.2) ∧ p.2 ∈ rewrittenPageSeedKeys P R m G ∧
        ¬ (∃ k, P.claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ P.aliasEdges G, σ e.2 = p.2)) := by
  have hd : P.declaredKey (renameBy R m G) p.2 = true ↔
      ∃ e ∈ P.aliasEdges G, σ e.2 = p.2 := by
    rw [P.declaredKey_iff]
    simp only [aliasEdges_merge H, Reach.mapE, List.mem_map]
    constructor
    · rintro ⟨e', ⟨e, he, rfl⟩, hk⟩; exact ⟨e, he, hk⟩
    · rintro ⟨e, he, hk⟩; exact ⟨_, ⟨e, he, rfl⟩, hk⟩
  simp only [Decided.pages, P.mem_pagesR, real_merge H, pageSeedKeys_merge_exact H,
    Bool.true_and, Bool.eq_false_iff, ne_eq, claimedKey_merge H, hd]

end Params
end CM
