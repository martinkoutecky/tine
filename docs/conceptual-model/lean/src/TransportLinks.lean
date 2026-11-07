/- T6 exact transport and generic excluded-page lemmas. The public configured
Favorites semantics is Decided.Link; its fixed-name transport is proved below.
The two-page-parameter lemmas are mathematical helpers, not a product policy. -/
import MergeRename
namespace CM
namespace Params
variable {K : Type} [DecidableEq K] {P : Params K}
variable {R : CM.Rewriter} {m : String → String} {σ : K → K}

/-- Exact link condition under a merge, for a real source file f. Endpoint
inequality remains in the formula; a merged edge can become an excluded self-link.
The inner predicate uses old occurrences and mapped old alias edges (T3 exact). -/
theorem link_merge_exact {G : Graph} {fav' : Page K}
    (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    P.Link true fav' (renameBy R m G) (σp σ (P.claim f)) t ↔
      σp σ (P.claim f) ∈ P.pagesR true (renameBy R m G) ∧
      t ∈ P.pagesR true (renameBy R m G) ∧
      σp σ (P.claim f) ≠ t ∧ σp σ (P.claim f) ≠ fav' ∧
      ∃ b ∈ P.pageBlocks G (P.claim f),
        ∃ o ∈ P.occAt G b, ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := by
  unfold Link
  rw [pageBlocks_merge_real H hf]
  have hx : (∃ b' ∈ (P.pageBlocks G (P.claim f)).map (blockMap R m G),
      P.References (renameBy R m G) b' t) ↔
      ∃ b ∈ P.pageBlocks G (P.claim f), ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := by
    constructor
    · rintro ⟨b', hb', hr⟩
      obtain ⟨b, hb, rfl⟩ := List.mem_map.mp hb'
      obtain ⟨g, ho, hp, hi⟩ := P.mem_pageBlocks.mp hb
      have hg := (P.owner_spec ho).1
      have he : b = (g.path, b.2) := Prod.ext hp rfl
      rw [he, blockMap_eq H.wf hg] at hr
      refine ⟨b, hb, ?_⟩
      rw [he]
      exact (references_merge_exact H hg _ t).mp hr
    · rintro ⟨b, hb, hr⟩
      obtain ⟨g, ho, hp, hi⟩ := P.mem_pageBlocks.mp hb
      have hg := (P.owner_spec ho).1
      have he : b = (g.path, b.2) := Prod.ext hp rfl
      refine ⟨blockMap R m G b, List.mem_map_of_mem hb, ?_⟩
      rw [he, blockMap_eq H.wf hg]
      exact (references_merge_exact H hg _ t).mpr (by rw [he] at hr; exact hr)
  rw [hx]
/-- **T6 (rename), replacement claim.** Under T3's hypotheses, with the whole graph exception-free and
the Favorites exclusion transported (`hfav`: `σp` sends exactly `fav` to `fav'`), `link` is carried by `σp`. -/
theorem link_rename_favorites {r : Bool} {fav fav' : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav' ↔ p = fav)
    {S T : Page K} (hS : S ∈ P.pagesR r G) (hT : T ∈ P.pagesR r G) :
    P.Link r fav G S T ↔ P.Link r fav' (renameBy R m G) (σp σ S) (σp σ T) := by
  have hSp := P.inPlay_of_pagesR hS
  have hTp := P.inPlay_of_pagesR hT
  unfold Link
  rw [pageBlocks_rename H hSp]
  have hne : S ≠ T ↔ σp σ S ≠ σp σ T :=
    ⟨fun h e => h (pagesR_rename_inj H hS hT e), fun h e => h (e ▸ rfl)⟩
  have hf : S ≠ fav ↔ σp σ S ≠ fav' := ⟨fun h e => h ((hfav S hS).mp e), fun h e => h ((hfav S hS).mpr e)⟩
  have hr : (∃ b ∈ P.pageBlocks G S, P.References G b T) ↔
      ∃ b' ∈ (P.pageBlocks G S).map (blockMap R m G), P.References (renameBy R m G) b' (σp σ T) := by
    constructor
    · rintro ⟨b, hb, hrf⟩
      have hbB := P.pageBlocks_sub_blocks hb
      exact ⟨_, List.mem_map_of_mem hb, (references_rename H hbB (hx b hbB) hTp).mp hrf⟩
    · rintro ⟨b', hb', hrf⟩
      obtain ⟨b, hb, rfl⟩ := List.mem_map.mp hb'
      have hbB := P.pageBlocks_sub_blocks hb
      exact ⟨b, hb, (references_rename H hbB (hx b hbB) hTp).mpr hrf⟩
  rw [hne, hf, hr]
  constructor
  · rintro ⟨_, _, c, d, e⟩
    exact ⟨pagesR_rename_fwd H hx hS, pagesR_rename_fwd H hx hT, c, d, e⟩
  · rintro ⟨_, _, c, d, e⟩
    exact ⟨hS, hT, c, d, e⟩

/-- **T6 (near under rename).** For seeds and targets among the pages of `G`, the neighbourhoods are
carried by `σp`, in every direction. -/
theorem near_rename_favorites {r : Bool} {fav fav' : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav' ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ P.pagesR r G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ P.pagesR r G) :
    P.NearP r fav G d k X T ↔ P.NearP r fav' (renameBy R m G) d k (X.map (σp σ)) (σp σ T) := by
  have base : ∀ T ∈ P.pagesR r G, (T ∈ X ↔ σp σ T ∈ X.map (σp σ)) := fun T hT => by
    constructor
    · exact List.mem_map_of_mem
    · intro h
      obtain ⟨x, hx', e⟩ := List.mem_map.mp h
      rw [← pagesR_rename_inj H (hX x hx') hT e]; exact hx'
  have hout : ∀ k, ∀ T ∈ P.pagesR r G,
      (P.OutP r fav G k X T ↔ P.OutP r fav' (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [OutP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.1).mp hS, (link_rename_favorites H hx hfav hl.1 hT).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename_favorites H hx hfav hS hT).mpr hl⟩
  have hin : ∀ k, ∀ T ∈ P.pagesR r G,
      (P.InP r fav G k X T ↔ P.InP r fav' (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [InP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.2.1).mp hS, (link_rename_favorites H hx hfav hT hl.2.1).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.2.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename_favorites H hx hfav hT hS).mpr hl⟩
  cases d with
  | out => exact hout k T hT
  | inn => exact hin k T hT
  | both => simp only [NearP]; rw [hout k T hT, hin k T hT]

end Params

namespace Decided
open Params
variable {K : Type} [DecidableEq K] {P : Params K}
variable {R : CM.Rewriter} {m : String → String} {σ : K → K}

/-- Configured-name exclusion is kept fixed. The extra condition says exactly
which old source keys map to the configured key; config itself never moves. -/
theorem link_rename_config {config : Config} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G)
    (hconfig : ∀ p ∈ pages P G,
      σ p.2 = P.key config.favoritesPage ↔ p.2 = P.key config.favoritesPage)
    {S T : Page K} (hS : S ∈ pages P G) (hT : T ∈ pages P G) :
    Link P config G S T ↔ Link P config (renameBy R m G) (σp σ S) (σp σ T) := by
  apply link_rename_favorites H hx (fav := (S.1, P.key config.favoritesPage))
    (fav' := (S.1, P.key config.favoritesPage)) ?_ hS hT
  intro p hp
  simp only [σp, Prod.ext_iff, hconfig p hp]

/-- Exact configured links retain the post-rename fixed-key exclusion and
endpoint inequality. Page membership uses the enlarged CM3-3 universe. -/
theorem link_merge_config_exact {config : Config} {G : Graph}
    (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    Link P config (renameBy R m G) (σp σ (P.claim f)) t ↔
      σp σ (P.claim f) ∈ pages P (renameBy R m G) ∧
      t ∈ pages P (renameBy R m G) ∧
      σp σ (P.claim f) ≠ t ∧ σ (P.claim f).2 ≠ P.key config.favoritesPage ∧
      ∃ b ∈ P.pageBlocks G (P.claim f), ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := by
  simpa only [Link, pages, σp, ne_eq, Prod.mk.injEq, true_and] using
    (link_merge_exact (fav' := ((P.claim f).1, P.key config.favoritesPage)) H hf t)

theorem near_rename_config {config : Config} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hconfig : ∀ p ∈ pages P G, σ p.2 = P.key config.favoritesPage ↔ p.2 = P.key config.favoritesPage)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ pages P G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ pages P G) :
    NearP P config G d k X T ↔ NearP P config (renameBy R m G) d k (X.map (σp σ)) (σp σ T) := by
  have base : ∀ T ∈ pages P G, (T ∈ X ↔ σp σ T ∈ X.map (σp σ)) := fun T hT => by
    constructor
    · exact List.mem_map_of_mem
    · intro h
      obtain ⟨x, hx', e⟩ := List.mem_map.mp h
      rw [← pagesR_rename_inj H (hX x hx') hT e]; exact hx'
  have hout : ∀ k, ∀ T ∈ pages P G,
      (OutP P config G k X T ↔ OutP P config (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [OutP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.1).mp hS, (link_rename_config H hx hconfig hl.1 hT).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename_config H hx hconfig hS hT).mpr hl⟩
  have hin : ∀ k, ∀ T ∈ pages P G,
      (InP P config G k X T ↔ InP P config (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [InP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.2.1).mp hS, (link_rename_config H hx hconfig hT hl.2.1).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.2.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename_config H hx hconfig hT hS).mpr hl⟩
  cases d with
  | out => exact hout k T hT
  | inn => exact hin k T hT
  | both => simp only [NearP]; rw [hout k T hT, hin k T hT]

end Decided
end CM
