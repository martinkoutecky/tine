/- Round 8 (D-20261102-MC, D-20261102-MC5): Martin's merge rulings.

* M-Q7 (merge into a journal: allowed). `Merge7.MergeBase` holds for a journal
  survivor; every round-7 lemma stated over it (blocks, owners, alias edges,
  references, Linked References) and `Merge7b.bodies_kept` (over
  `MergeBodyHyps`) now cover it. This module adds the real-page and page-universe
  theorems for a survivor of either kind (`real_merge8`, `pages_merge8`; round 7's
  `real_merge7`/`pages_merge7` stated the page case through `σp σ (claim S) =
  claim D`, false for a journal) and `journal_references`.
* M-Q5(a) (rename onto an alias-only name, answer "merge into the owner"). The
  rewriter sends the source's name to the typed alias (`m old = new`, key map
  `σ`), while the source's page becomes the owner's (key map `τ`, with
  `τ (key S) = key D`). The two differ only at the source key; the owner's alias
  edge `(key D, σ (key S))` bridges them (`via_bridge`). Round 8b: the merge
  hypotheses are `MergeBase … τ … σ` (page fields under `τ`, alias contracts
  under the text map `σ`: an `alias:: src` anywhere, also in the source or the
  owner, is rewritten to `alias:: new`); `cls_split` shows the after-graph's
  alias classes are still those of `mapE τ`. Gap C11 is closed for the
  references and alias-class statements: `references_forward_via`,
  `references_to_source_via`, `linked_references_via`, `class_join_via` (MA1–MA4),
  plus the τ-instances of `no_block_lost`/`bodies_kept`/`real_merge8`; it covers
  the verifier's three accepted merges where an alias spells the source
  (`MergeWitness8b`). Not covered: page-name query leaves (MW18; round 9 rewrites
  them to the owner's name, `Rulings9.merge9`, Q9-8) and `pages_merge8` (stated
  for one map).
* M-Q5(b) (answer "make it its own page, still an alias of the owner"): the
  decided rename (`Decided.renameAllowed`, rule 6; round 9 adds the CM-OWN-CHILD
  refusal, and `Rulings9.rename_onto_alias_decided` drops the fixed-alias
  premise of `rename_onto_alias` below). Its transport is
  `MergeRename` (`Params.MergeHyps`: the source key and the alias-only key merge
  under `σ`). `rename_onto_alias`: the source becomes the real page `new`, `new`
  stays in the owner's alias class, the owner stays real, and references to the
  source, to `new` and to the owner all count toward the owner's class. Its
  premise `σ (σ (key S)) = σ (key S)` excludes `new` inside the source's
  namespace subtree: there the guard accepts but the owner's `alias:: new` is
  rewritten to `new/…` and the outcome fails (verifier finding 1,
  `MergeWitness8b.mw_rename_alias_child`; open, for Martin).
* M-Q6 (merge into an own namespace child: refused) is in
  `Merge7.mergeAllowed` / `Merge7.merge_outside_subtree`. -/
import Merge7b

set_option linter.unusedSectionVars false
namespace CM
namespace Merge8
open Params Merge7 Merge7b

variable {K : Type} [DecidableEq K] {P : Params K}
  {R : CM.Rewriter} {J : Merger} {m : String → String} {σ τ σa : K → K} {S D : File} {G : Graph}

/-! ## M-Q7: a survivor of either kind (page or journal) -/

theorem claim_ne_src (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ G) (hfS : f ≠ S) :
    P.claim f ≠ P.claim S :=
  fun e => hfS (H.srcUniq f hf (congrArg Prod.snd e))

theorem σp_dst8 (H : MergeBase P R J m σ S D G σa) : σp σ (P.claim D) = P.claim D := by
  simp only [σp]; rw [H.dstKey]

/-- **MJ1 (real pages, either survivor kind).** The after-graph's real pages
are exactly the σ-images of the before-graph's real pages other than the
source's. With a page survivor this is `real_merge7`; with a journal survivor
the source's page `(page, key S)` is gone and its key is now the journal's. -/
theorem real_merge8 (H : MergeBase P R J m σ S D G σa) (q : Page K) :
    P.real (mergeBy R J m S D G) q = true ↔
      ∃ p, P.real G p = true ∧ p ≠ P.claim S ∧ σp σ p = q := by
  rw [P.real_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · exact ⟨P.claim D, P.real_iff.mpr ⟨D, H.dst, rfl⟩, claim_ne_src H H.dst (fun e => H.ne e.symm),
        by rw [σp_dst8 H, ← claim_survivor H, e]⟩
    · have hf' := mem_others.mp hf
      exact ⟨P.claim f, P.real_iff.mpr ⟨f, hf'.1, rfl⟩, claim_ne_src H hf'.1 hf'.2.1,
        by rw [← claim_apply H.laws hf, e]⟩
  · rintro ⟨p, hp, hpS, rfl⟩
    obtain ⟨f, hf, rfl⟩ := P.real_iff.mp hp
    have hS : f ≠ S := fun e => hpS (by rw [e])
    by_cases hD : f = D
    · subst hD; exact ⟨_, survivor_mem H, by rw [claim_survivor H, σp_dst8 H]⟩
    have ho : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
    exact ⟨_, other_mem ho, claim_apply H.laws ho⟩

/-- **MJ2 (exact page universe, either survivor kind).** As `pages_merge7`, with
the real part from `real_merge8`. -/
theorem pages_merge8 (H : MergeBase P R J m σ S D G) (p : Page K) :
    p ∈ Decided.pages P (mergeBy R J m S D G) ↔
      (∃ q, P.real G q = true ∧ q ≠ P.claim S ∧ σp σ q = p) ∨
      (p = (P.refKind p.2, p.2) ∧
        (p.2 ∈ rewrittenRefKeys P R m G ∨ p.2 ∈ keptNameKeys P R m S G) ∧
        ¬ (∃ k, P.claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ P.aliasEdges G, σ e.2 = p.2)) := by
  have hd : P.declaredKey (mergeBy R J m S D G) p.2 = true ↔
      ∃ e ∈ P.aliasEdges G, σ e.2 = p.2 := by
    rw [P.declaredKey_iff]
    simp only [aliasEdges_merge7 H, Reach.mapE, List.mem_map]
    constructor
    · rintro ⟨e', ⟨e, he, rfl⟩, hk⟩; exact ⟨e, he, hk⟩
    · rintro ⟨e, he, hk⟩; exact ⟨_, ⟨e, he, rfl⟩, hk⟩
  have hs : p.2 ∈ P.pageSeedKeys (mergeBy R J m S D G) ↔
      (p.2 ∈ rewrittenRefKeys P R m G ∨ p.2 ∈ keptNameKeys P R m S G) := by
    simp only [pageSeedKeys, List.mem_append, refKeys_merge7 H, nameKeys_merge7 H]
  simp only [Decided.pages, P.mem_pagesR, real_merge8 H, hs, Bool.true_and, Bool.eq_false_iff,
    ne_eq, claimedKey_merge7 H, hd]

theorem claim_journal {D : File} (hj : kindOf D = .journal) : P.claim D = (.journal, (P.claim D).2) := by
  unfold claim; rw [hj]

/-- **MJ3 (merge into a journal: references follow).** A block that referenced
the merged page has an image block that references the journal; the journal
owns its page afterwards (`owner_dst`), every body block of both files is kept
in it (`bodies_kept` over `MergeBodyHyps … σ σ`), and its Linked References are
`linked_references_merge7` (stated over `MergeBase`). -/
theorem journal_references (H : MergeBase P R J m σ S D G) (hj : kindOf D = .journal)
    {b : BlockId} (hb : b ∈ P.blocks G) (hns : NsForward P R m σ (P.occAt G b))
    (hr : P.References G b (P.claim S)) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (.journal, (P.claim D).2) ∧
      P.real (mergeBy R J m S D G) (.journal, (P.claim D).2) = true := by
  obtain ⟨b', hb', hi, hr'⟩ := references_to_source H hb hns hr
  refine ⟨b', hb', hi, by rw [claim_journal hj] at hr'; exact hr', ?_⟩
  exact (real_merge8 H _).mpr ⟨P.claim D, P.real_iff.mpr ⟨D, H.dst, rfl⟩,
    claim_ne_src H H.dst (fun e => H.ne e.symm), by rw [σp_dst8 H, ← claim_journal hj]⟩

/-! ## M-Q5(a): merge into the owner of the alias `new` -/

/-- How the text key map `σ` (the rename map, `σ (key S) = key new`) and the
merge's key map `τ` (`τ (key S) = key D`, the owner) relate: equal off the
source key; the owner declares `new` (`mergeTarget_alias_edge`); `τ` fixes
`new`'s key. -/
structure ViaAlias (P : Params K) (σ τ : K → K) (S D : File) (G : Graph) : Prop where
  agree : ∀ k, k ≠ (P.claim S).2 → τ k = σ k
  declares : ((P.claim D).2, σ (P.claim S).2) ∈ P.aliasEdges G
  fixed : τ (σ (P.claim S).2) = σ (P.claim S).2

/-- **MA0 (the bridge).** In the after-graph's alias graph, every key's text
image is in the class of its page image. -/
theorem via_bridge (H : MergeBase P R J m τ S D G σa) (V : ViaAlias P σ τ S D G) (k : K) :
    cls (Reach.mapE τ (P.aliasEdges G)) (σ k) (τ k) := by
  by_cases hk : k = (P.claim S).2
  · subst hk
    have he : ((P.claim D).2, σ (P.claim S).2) ∈ Reach.mapE τ (P.aliasEdges G) := by
      have := List.mem_map_of_mem (f := fun e : K × K => (τ e.1, τ e.2)) V.declares
      simp only [H.dstKey, V.fixed] at this
      exact this
    rw [H.srcKey]; exact Reach.edge' he
  · rw [V.agree k hk]; exact cls_self _ _

/-- Round 8b: a walk transfers to another edge list when each of its edges does. -/
theorem reach_sim {E E' : List (K × K)} (h : ∀ e ∈ E, Reach E' e.1 e.2) {a b : K}
    (r : Reach E a b) : Reach E' a b := by
  induction r with
  | refl => exact Reach.refl _
  | fwd _ e ih => exact Reach.trans ih (h _ e)
  | bwd _ e ih => exact Reach.trans ih (Reach.symm (h _ e))

/-- **MA0b (round 8b: the bridge in the after-graph's own alias edges).** The
after-graph's alias edges are the before-graph's with the declaring owner's key
under the page map `τ` and the alias key under the text map `σ`
(`aliasEdges_split`); in them, too, every key's text image is in the class of
its page image. -/
theorem bridge_split (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G) (k : K) :
    cls ((P.aliasEdges G).map (fun e => (τ e.1, σ e.2))) (σ k) (τ k) := by
  by_cases hk : k = (P.claim S).2
  · subst hk
    have hne : σ (P.claim S).2 ≠ (P.claim S).2 := by
      intro e
      have h1 := V.fixed
      rw [e, H.srcKey] at h1
      exact H.ne (H.srcUniq D H.dst h1).symm
    have hσσ : σ (σ (P.claim S).2) = σ (P.claim S).2 := by
      rw [← V.agree _ hne]; exact V.fixed
    have he : ((P.claim D).2, σ (P.claim S).2) ∈ (P.aliasEdges G).map (fun e => (τ e.1, σ e.2)) := by
      have := List.mem_map_of_mem (f := fun e : K × K => (τ e.1, σ e.2)) V.declares
      simp only [H.dstKey, hσσ] at this
      exact this
    rw [H.srcKey]; exact Reach.edge' he
  · rw [V.agree k hk]; exact cls_self _ _

/-- **MA0c (round 8b: alias classes after the alias-owner merge).** With the
alias contracts read under the text map `σ` (every `alias:: src`, wherever it
is declared, now spells `new`), the after-graph's alias classes are exactly
those of the before-graph's alias edges mapped by the page map `τ`. -/
theorem cls_split (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G) (x y : K) :
    P.aliasClass (mergeBy R J m S D G) x y ↔ cls (Reach.mapE τ (P.aliasEdges G)) x y := by
  unfold aliasClass cls
  rw [Reach.congr (aliasEdges_split H)]
  constructor
  · intro r
    refine reach_sim (fun e he => ?_) r
    obtain ⟨⟨a, b⟩, hab, rfl⟩ := List.mem_map.mp he
    show Reach _ (τ a) (σ b)
    exact Reach.trans (Reach.edge (List.mem_map_of_mem (f := fun e : K × K => (τ e.1, τ e.2)) hab))
      (Reach.symm (via_bridge H V b))
  · intro r
    refine reach_sim (fun e he => ?_) r
    obtain ⟨⟨a, b⟩, hab, rfl⟩ := List.mem_map.mp he
    show Reach _ (τ a) (τ b)
    exact Reach.trans (Reach.edge (List.mem_map_of_mem (f := fun e : K × K => (τ e.1, σ e.2)) hab))
      (bridge_split H V b)

/-- **MA1 (references follow the alias-owner merge).** Under the forward
namespace condition for the TEXT map `σ`, a block that referenced `t` has an
image block that references `τ t`. -/
theorem references_forward_via (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G)
    {b : BlockId} (hb : b ∈ P.blocks G) {t : Page K} (hns : NsForward P R m σ (P.occAt G b))
    (hr : P.References G b t) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (σp τ t) :=
  references_forward_cls_in (k' := τ t.2) H (cls_split H V) hb (fun o ho u hu hc => by
    obtain ⟨v, hv, he⟩ := hns o ho u hu
    exact ⟨v, hv, by rw [he]; exact Reach.trans (via_bridge H V _) (Reach.map_fwd τ hc)⟩) hr

/-- **MA2 (C11: references to the source count toward the owner).** -/
theorem references_to_source_via (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G)
    {b : BlockId} (hb : b ∈ P.blocks G) (hns : NsForward P R m σ (P.occAt G b))
    (hr : P.References G b (P.claim S)) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (P.claim D) := by
  obtain ⟨b', hb', hi, hr'⟩ := references_forward_via H V hb hns hr
  exact ⟨b', hb', hi, (references_key (t := σp τ (P.claim S)) (t' := P.claim D) H.srcKey).mp hr'⟩

/-- **MA3 (Linked References of the owner).** Forward: every block that
referenced the source or the owner has an image referencing the owner. Backward
(exact): every after-graph block referencing the owner is the image of an old
block one of whose rewritten occurrences reaches the owner's key in the
after-graph's alias graph, whose classes are those of `mapE τ` (`cls_split`). -/
theorem linked_references_via (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G) :
    (∀ b ∈ P.blocks G, NsForward P R m σ (P.occAt G b) →
      (P.References G b (P.claim S) ∨ P.References G b (P.claim D)) →
      ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
        P.References (mergeBy R J m S D G) b' (P.claim D)) ∧
    (∀ b' ∈ P.blocks (mergeBy R J m S D G), P.References (mergeBy R J m S D G) b' (P.claim D) →
      ∃ b ∈ P.blocks G, Img R J m S D G b b' ∧ ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
          cls (Reach.mapE τ (P.aliasEdges G)) (P.key u) (P.claim D).2) := by
  refine ⟨fun b hb hns hr => ?_, linked_references_back_in H (cls_split H V)⟩
  rcases hr with hr | hr
  · exact references_to_source_via H V hb hns hr
  · obtain ⟨b', hb', hi, hr'⟩ := references_forward_via H V hb hns hr
    exact ⟨b', hb', hi, (references_key (t := σp τ (P.claim D)) (t' := P.claim D) H.dstKey).mp hr'⟩

/-- **MA4 (alias classes join, text keys).** Every key of the source's or the
owner's class, as the rewritten text spells it (`σ`), is in the owner's class
afterwards; in particular `new` (`σ (key S)`) stays an alias of the owner. -/
theorem class_join_via (H : MergeBase P R J m τ S D G σ) (V : ViaAlias P σ τ S D G) {x : K}
    (hx : cls (P.aliasEdges G) (P.claim S).2 x ∨ cls (P.aliasEdges G) (P.claim D).2 x) :
    cls (P.aliasEdges (mergeBy R J m S D G)) (P.claim D).2 (σ x) := by
  refine (cls_split H V _ _).mpr ?_
  have h1 : cls (Reach.mapE τ (P.aliasEdges G)) (P.claim D).2 (τ x) := by
    rcases hx with hx | hx
    · have := Reach.map_fwd τ hx; rwa [H.srcKey] at this
    · have := Reach.map_fwd τ hx; rwa [H.dstKey] at this
  exact Reach.trans h1 (Reach.symm (via_bridge H V x))

/-- **MA5 (the decided merge's alias target declares `new`).** When
`mergeTarget` resolves `new` through an alias, the survivor is a declaring
owner of `new`'s key: the alias edge `ViaAlias.declares` needs. -/
theorem mergeTarget_alias_edge {old new : String} {D : File}
    (h : mergeTarget P G old new = some D) (hk : (P.claim D).2 ≠ P.key new) :
    ((P.claim D).2, P.key new) ∈ P.aliasEdges G := by
  unfold mergeTarget at h
  split at h
  · cases h
  · split at h
    · next f hc =>
      cases h
      have : D ∈ G.filter (fun g => decide ((P.claim g).2 = P.key new)) := by
        rw [hc]; exact List.mem_singleton_self _
      exact absurd (by simpa using (List.mem_filter.mp this).2) hk
    · split at h
      · next f hc2 =>
        cases h
        have : D ∈ (P.declaringOwners G).filter (fun g => P.declaresKey g (P.key new)) := by
          rw [hc2]; exact List.mem_singleton_self _
        obtain ⟨hD, hdk⟩ := List.mem_filter.mp this
        obtain ⟨s, hs, hsk⟩ := List.any_eq_true.mp hdk
        exact P.mem_aliasEdges.mpr ⟨D, hD, rfl, s, hs, by simpa using hsk⟩
      · cases h
    · cases h

/-! ## M-Q5(b): rename onto an alias-only name (the decided rename) -/

/-- Rule 6.4's class clause never fires when `new` names no real page (an
alias-only name). -/
theorem classClash_unclaimed {old new : String} (h : P.claimedKey G (P.key new) = false) :
    Decided.classClash P G old new = false := by
  simp [Decided.classClash, h]

/-- References are read through alias classes: two pages of one class have
the same Linked References. -/
theorem references_same_class {G : Graph} {b : BlockId} {t t' : Page K}
    (h : cls (P.aliasEdges G) t.2 t'.2) : P.References G b t ↔ P.References G b t' := by
  rw [P.references_iff_keys, P.references_iff_keys]
  constructor
  · rintro ⟨s, hs, hc⟩; exact ⟨s, hs, Reach.trans hc h⟩
  · rintro ⟨s, hs, hc⟩; exact ⟨s, hs, Reach.trans hc (Reach.symm h)⟩

/-- **MB1 (rename onto an alias-only name).** `S` (a page) is renamed to `new`,
whose key `a = σ (key S)` the owner `D` declares as an alias and no file claims.
Under the rename hypotheses of `MergeRename` (`σ` may merge the source key with
the alias-only key `a`; real keys stay injective), and with `σ` fixing the
owner's key and `a`: afterwards `new` is a real page (AL1: its own file), it is
still in the owner's alias class, the owner is still real, and every block
whose text goes forward and that referenced the source, `new`, or anything in
the owner's class, references both `new` and the owner (one class, AL2). -/
theorem rename_onto_alias (H : P.MergeHyps R m σ G) {S D : File} (hS : S ∈ G) (hD : D ∈ G)
    (hSp : kindOf S = .page) (he : ((P.claim D).2, σ (P.claim S).2) ∈ P.aliasEdges G)
    (hDk : σ (P.claim D).2 = (P.claim D).2) (ha : σ (σ (P.claim S).2) = σ (P.claim S).2) :
    P.real (renameBy R m G) (.page, σ (P.claim S).2) = true ∧
    P.real (renameBy R m G) (P.claim D) = true ∧
    cls (P.aliasEdges (renameBy R m G)) (P.claim D).2 (σ (P.claim S).2) ∧
    ∀ f ∈ G, ∀ i, (∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2,
        ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, P.key v = σ (P.key u)) →
      (P.References G (f.path, i) (P.claim S) ∨
        P.References G (f.path, i) (.page, σ (P.claim S).2) ∨
        P.References G (f.path, i) (P.claim D)) →
      P.References (renameBy R m G) ((R.apply m f).path, i) (.page, σ (P.claim S).2) ∧
      P.References (renameBy R m G) ((R.apply m f).path, i) (P.claim D) := by
  have hcls : cls (P.aliasEdges (renameBy R m G)) (P.claim D).2 (σ (P.claim S).2) := by
    unfold cls
    rw [Reach.congr (aliasEdges_merge H)]
    have := List.mem_map_of_mem (f := fun e : K × K => (σ e.1, σ e.2)) he
    simp only [hDk, ha] at this
    exact Reach.edge this
  have hσD : σp σ (P.claim D) = P.claim D := by simp only [σp]; rw [hDk]
  have hσS : σp σ (P.claim S) = (.page, σ (P.claim S).2) := by
    have e1 : (P.claim S).1 = .page := by unfold claim; rw [hSp]
    simp only [σp]; rw [e1]
  refine ⟨(real_merge H _).mpr ⟨P.claim S, P.real_iff.mpr ⟨S, hS, rfl⟩, hσS⟩,
    (real_merge H _).mpr ⟨P.claim D, P.real_iff.mpr ⟨D, hD, rfl⟩, hσD⟩, hcls, ?_⟩
  intro f hf i hns hr
  have both : P.References (renameBy R m G) ((R.apply m f).path, i) (.page, σ (P.claim S).2) ↔
      P.References (renameBy R m G) ((R.apply m f).path, i) (P.claim D) :=
    references_same_class (Reach.symm hcls)
  have hfwd := fun {t : Page K} (h : P.References G (f.path, i) t) =>
    references_merge_forward H hf i hns h
  rcases hr with hr | hr | hr
  · have h1 := hfwd hr; rw [hσS] at h1; exact ⟨h1, both.mp h1⟩
  · have h1 := hfwd hr; simp only [σp] at h1; rw [ha] at h1; exact ⟨h1, both.mp h1⟩
  · have h1 := hfwd hr; rw [hσD] at h1; exact ⟨both.mpr h1, h1⟩

end Merge8
end CM
