/- Round 12: D-20261105-CM12, alias-only source names.
The operation API remains Rulings11.offers/answer, extended in place.
No parser, key normalizer, or production text writer is asserted here. -/
import Witness11

set_option linter.unusedSectionVars false
namespace CM
namespace Rulings12
open Params Merge7 Rulings9 Ref10 Rulings11 QueryRename6

variable {K : Type} [DecidableEq K] (P : Params K)

theorem aliasSource_iff (G : Graph) (old : String) :
    aliasSource P G old = true ↔
      P.claimedKey G (P.key old) = false ∧ P.declaredKey G (P.key old) = true := by
  simp [aliasSource]

theorem aliasSource_no_ref {G : Graph} {old : String} (h : aliasSource P G old = true) :
    refSource P G old = false := by
  have hd := ((aliasSource_iff P G old).mp h).2
  simp [refSource, refOnlyPagesR, hd]

theorem aliasSource_no_file {G : Graph} {old : String} (h : aliasSource P G old = true) :
    mergeSource P G old = none := by
  cases hs : mergeSource P G old with
  | none => rfl
  | some S => have hf := aliasSource_none_of_mergeSource P hs; rw [h] at hf; cases hf

theorem aliasSource_kind {G : Graph} {old : String} (h : aliasSource P G old = true) :
    sourceKind P G old = .aliasOnly := by
  simp [sourceKind, aliasSource_no_file P h, aliasSource_no_ref P h, h]

theorem aliasSource_no_own {G : Graph} {old new : String} (h : aliasSource P G old = true) :
    Choice.ownPage ∉ offers P G old new := by
  simp [offers, ownReady, noFileSource, h]
  split <;> simp <;> split <;> simp <;> split <;> simp

theorem aliasSource_no_plain_alias {G : Graph} {old new : String}
    (h : aliasSource P G old = true) (hd : P.declaredKey G (P.key new) = true)
    (hne : P.key new ≠ P.key old) :
    Choice.rename ∉ offers P G old new := by
  simp [offers, plainReady, noFileSource, h, hd, hne]
  split <;> simp <;> split <;> simp <;> split <;> simp

theorem aliasSource_offer_set {G : Graph} {old new : String}
    (h : aliasSource P G old = true) (hd : (dialogOwner P G old new).isSome = true)
    (ha : mergeReady P G old new = true) :
    offers P G old new = [.mergeOwner, .cancel] := by
  simp [offers, hd, ownReady, noFileSource, h, ha]

/-- New plain rename, exactly the shared guard, not an alias-text-only edit. -/
theorem aliasSource_rename (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : aliasSource P G old = true) :
    renameChosen P R G old new =
      if renameAllowedFree P G old new then
        some (renameBy R.base (nsSubst P.key P.urlHasOrigin old new) G) else none := by
  simp [renameChosen, h, Params.rename]

theorem renameChosen_eq10 (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : aliasSource P G old = false) :
    renameChosen P R G old new = rename10 P R G old new := by
  simp [renameChosen, h]

theorem renameChosen_free (R : Decided.Rewriter) {G G' : Graph} {old new : String}
    (h : renameChosen P R G old new = some G') :
    renameAllowedFree P G old new = true ∧
      G' = renameBy R.base (nsSubst P.key P.urlHasOrigin old new) G := by
  unfold renameChosen at h
  split at h
  · split at h
    · next ha => exact ⟨ha, (Option.some.inj h).symm⟩
    · cases h
  · exact rename10_some_free P R h

theorem aliasSource_merge_guard {G : Graph} {old new : String} {D : File}
    (h : aliasSource P G old = true) (ht : target P G old new = some D) :
    mergeReady P G old new = noFileMergeCore P G old new D := by
  simp [mergeReady, ht, noFileSource, h]

theorem aliasSource_merge (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} {D : File}
    (h : aliasSource P G old = true) (ht : target P G old new = some D)
    (ha : mergeReady P G old new = true) :
    mergeChosen P R J G old new = some
      (renameBy (R.at (queryMap P old new D)).base (nsSubst P.key P.urlHasOrigin old new) G) := by
  simp [mergeChosen, ht, ha, noFileSource, h]

/-- A whole offered mutation for an alias-only source is always a rewrite of
every original file; no source content is deleted or appended to the target. -/
theorem aliasSource_answer_structure (R : QRewriter) (J : (String → String) → Merger)
    {G G' : Graph} {old new : String} {c : Choice}
    (hs : aliasSource P G old = true) (hc : c ≠ .cancel)
    (ha : answer P R J G old new c = some G') :
    ∃ mq, G' = renameBy (R.at mq).base (nsSubst P.key P.urlHasOrigin old new) G ∧
      ((c = .rename ∧ mq = nsSubst P.key P.urlHasOrigin old new ∧
          renameAllowedFree P G old new = true) ∨
       (c = .mergeOwner ∧ ∃ D, target P G old new = some D ∧
          mq = queryMap P old new D ∧ noFileMergeCore P G old new D = true)) := by
  unfold answer at ha
  split at ha
  · next ho =>
    cases c with
    | cancel => exact False.elim (hc rfl)
    | ownPage =>
      have hn := aliasSource_no_own P (new := new) hs
      have hm : Choice.ownPage ∈ offers P G old new := by simpa using ho
      exact False.elim (hn hm)
    | rename =>
      obtain ⟨hg, he⟩ := renameChosen_free P _ ha
      exact ⟨_, he, Or.inl ⟨rfl, rfl, hg⟩⟩
    | mergeOwner =>
      have hf : noFileSource P G old = true := by simp [noFileSource, hs]
      obtain ⟨D, ht, hr, he⟩ := mergeChosen_structure P R J ha
      have hg := (aliasSource_merge_guard P hs ht).symm.trans hr
      rcases he with he | ⟨S, hS, _⟩
      · exact ⟨_, he.2, Or.inr ⟨rfl, D, ht, rfl, hg⟩⟩
      · rw [aliasSource_no_file P hs] at hS; cases hS
  · cases ha

theorem aliasSource_answer_length (R : QRewriter) (J : (String → String) → Merger)
    {G G' : Graph} {old new : String} {c : Choice}
    (hs : aliasSource P G old = true) (ha : answer P R J G old new c = some G') :
    G'.length = G.length := by
  by_cases hc : c = .cancel
  · subst c; rw [answer_cancel] at ha; cases ha; rfl
  · obtain ⟨mq, rfl, _⟩ := aliasSource_answer_structure P R J hs hc ha
    simp [renameBy]

/-- Every offered choice for the new source executes, and no other choice does.
This includes the offered plain rename, rather than just its low-level guard. -/
theorem aliasSource_answer_isSome (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} (hs : aliasSource P G old = true) (c : Choice) :
    (answer P R J G old new c).isSome = (offers P G old new).contains c := by
  by_cases ho : c ∈ offers P G old new
  · have hot : (offers P G old new).contains c = true := by simpa using ho
    rw [hot]
    cases c with
    | cancel => simp [answer_cancel]
    | ownPage => exact False.elim (aliasSource_no_own P hs ho)
    | rename =>
      have hg : renameAllowedFree P G old new = true := by
        simp [offers] at ho
        split at ho
        · simp at ho
        · split at ho
          · simp at ho
          · split at ho
            · next hp =>
              simp only [plainReady, Bool.and_eq_true] at hp
              simpa [renameReady, noFileSource, hs] using hp.1.1
            · simp at ho
      simp [answer, ho, renameChosen, hs, hg]
    | mergeOwner =>
      have hg : mergeReady P G old new = true := by
        simp [offers] at ho
        split at ho
        · split at ho <;> simp_all
        · split at ho
          · split at ho <;> simp_all
          · split at ho <;> simp_all
      simp only [answer, hot, Bool.true_eq, ite_true]
      exact (mergeChosen_isSome P R J G old new).trans hg
  · have hot : (offers P G old new).contains c = false := by simpa using ho
    rw [answer_unoffered P R J ho, hot]; rfl

/-- The shared own-child rule gates both possible mutations for the new source. -/
theorem aliasSource_own_child {G : Graph} {old new : String}
    (hs : aliasSource P G old = true) (hc : Decided.ownChildUsed P G old new = true) :
    offers P G old new = [.cancel] := by
  have hn : (nsMatch P.key P.urlHasOrigin old new).isSome = true :=
    (Rulings9.ownChildUsed_iff P).mp hc |>.1
  simp [offers, mergeReady, renameReady, plainReady, ownReady, noFileSource, hs,
    renameAllowedFree, noFileMergeCore, hc, hn]
  split <;> cases target P G old new <;> simp_all

theorem aliasSource_journal_risk {G : Graph} {old new : String}
    (hs : aliasSource P G old = true) (hj : Decided.JournalRisk P G old new) :
    offers P G old new = [.cancel] := by
  have hr := (Decided.journalRewriteRisks_nonempty P).mpr hj
  simp [offers, mergeReady, renameReady, plainReady, ownReady, noFileSource, hs,
    renameAllowedFree, noFileMergeCore, hr]
  split <;> cases target P G old new <;> simp_all

/-! Exact semantic consequences for either of the two maps chosen above.
These apply to alias-only sources, including multiple declarers and class
coalescence; no real-source premise is introduced. -/

variable {P} {R : CM.Rewriter} {m : String → String} {σ : K → K} {G : Graph}

/-- Every original declaration is rewritten, even when more than one page
declares old. Owner selection for navigation is not mutation target selection. -/
theorem all_alias_declarations (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G) :
    R.apply m f ∈ renameBy R m G ∧
      P.aliasDecl (R.apply m f).format (R.apply m f).text = (P.aliasDecl f.format f.text).map m :=
  ⟨List.mem_map_of_mem hf, H.laws.alias f hf⟩

theorem alias_group_exact (H : P.MergeHyps R m σ G) (a b : K) :
    cls (P.aliasEdges (renameBy R m G)) a b ↔ cls (Reach.mapE σ (P.aliasEdges G)) a b :=
  Reach.congr (P.aliasEdges_merge H)

theorem references_exact (H : P.MergeHyps R m σ G) {f : File} (hf : f ∈ G)
    (i : Nat) (t : Page K) :
    P.References (renameBy R m G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i), ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
        cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  P.references_merge_exact H hf i t

/-- Exact block-list transport, hence counting each block once keeps the
same ordering and multiplicity rather than merely a set correspondence. -/
theorem blocks_list (H : P.MergeHyps R m σ G) :
    P.blocks (renameBy R m G) = (P.blocks G).map (blockMap R m G) := by
  unfold blocks renameBy
  rw [List.flatMap_map, List.map_flatMap]
  apply congrArg List.flatten
  apply List.map_congr_left
  intro f hf
  have ho : P.isOwner (G.map (R.apply m)) (R.apply m f) = P.isOwner G f :=
    P.isOwner_merge H hf
  rw [ho, outline_length_rename H.laws hf]
  by_cases hown : P.isOwner G f = true
  · simp only [hown, ite_true, List.map_map]
    apply List.map_congr_left
    intro i _
    simp [Function.comp, blockMap_eq H.wf hf]
  · simp [hown]

def predictedRefs (P : Params K) (R : CM.Rewriter) (m : String → String)
    (σ : K → K) (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (P.occAt G b).any (fun o => (P.occClosure (rwOcc R.covers m o).2).any
    (fun u => sameCls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2))

def linkedBlocks (P : Params K) (G : Graph) (t : Page K) : List BlockId :=
  (P.blocks G).filter (fun b => P.referencesB G b t)

theorem referencesB_predicted (H : P.MergeHyps R m σ G) {b : BlockId}
    (hb : b ∈ P.blocks G) (t : Page K) :
    P.referencesB (renameBy R m G) (blockMap R m G b) t = predictedRefs P R m σ G b t := by
  obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
  have he : b = (f.path, b.2) := Prod.ext hp.symm rfl
  rw [he, blockMap_eq H.wf hf]
  have hi := P.references_merge_exact H hf b.2 t
  have hi' : P.referencesB (renameBy R m G) ((R.apply m f).path, b.2) t = true ↔
      predictedRefs P R m σ G (f.path, b.2) t = true := by
    rw [P.referencesB_iff, hi]
    simp [predictedRefs, List.any_eq_true, sameCls_iff]
  exact Bool.eq_iff_iff.mpr hi'

theorem linkedBlocks_exact (H : P.MergeHyps R m σ G) (t : Page K) :
    linkedBlocks P (renameBy R m G) t =
      ((P.blocks G).filter (fun b => predictedRefs P R m σ G b t)).map (blockMap R m G) := by
  unfold linkedBlocks
  rw [blocks_list H, List.filter_map]
  congr 1
  apply List.filter_congr
  intro b hb
  exact referencesB_predicted H hb t

theorem counts_exact (H : P.MergeHyps R m σ G) (t : Page K) :
    (linkedBlocks P (renameBy R m G) t).length =
      ((P.blocks G).filter (fun b => predictedRefs P R m σ G b t)).length := by
  rw [linkedBlocks_exact H, List.length_map]

/-- Independent query-map law: text/children take m, stored arguments take mq.
It does not promise arbitrary query denotation preservation through class unions. -/
def StoredLaw (P : Params K) {L : Type} (S : StoredParser L) (R : CM.Rewriter)
    (m mq : String → String) (G : Graph) : Prop :=
  ∀ f ∈ G, (P.outline (R.apply m f).format (R.apply m f).text).map
    (S.extract (R.apply m f).format) =
    ((P.outline f.format f.text).map (S.extract f.format)).map (List.map (renameQuery mq))

theorem stored_queries_exact {L : Type} (S : StoredParser L) (H : P.MergeHyps R m σ G)
    {mq : String → String} (hs : StoredLaw P S R m mq G) {f : File} (hf : f ∈ G) (i : Nat) :
    storedAt P S (renameBy R m G) ((R.apply m f).path, i) =
      (storedAt P S G (f.path, i)).map (renameQuery mq) := by
  rw [storedAt_some P S H.wf' (List.mem_map_of_mem hf), storedAt_some P S H.wf hf]
  have h := congrArg (fun xs => xs[i]?) (hs f hf)
  simp only [List.getElem?_map] at h
  rw [h]
  cases (P.outline f.format f.text)[i]? <;> rfl

end Rulings12
end CM
