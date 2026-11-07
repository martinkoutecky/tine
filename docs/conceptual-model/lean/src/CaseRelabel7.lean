/- Round 7 (D-20261102-CM4): a case-only rename usually MOVES the page file
(`pages/Foo.md` → `pages/foo.md`) while every key stays the same. T3's
`RwLaws.path` ("a file whose page does not move keeps its path") makes T3 with
σ = id inapplicable to such a rename. This module closes that gap without
changing T3: a pure path relabelling that keeps every file's claim, namespace
parents and owner order changes nothing but block ids (`relabel_*`), and a
same-key rename factors as T3 with the paths frozen followed by such a
relabelling (`same_key_moving_transport`). -/
import CaseRename7

namespace CM
namespace CaseRelabel7
open Params

theorem flatMap_map2 {α β γ : Type} (l : List α) (f : α → β) (g : β → List γ) :
    (l.map f).flatMap g = l.flatMap (fun a => g (f a)) := by
  induction l with
  | nil => rfl
  | cons a t ih => simp only [List.map_cons, List.flatMap_cons, ih]

theorem flatMap_congr2 {α β : Type} {l : List α} {f g : α → List β}
    (h : ∀ a ∈ l, f a = g a) : l.flatMap f = l.flatMap g := by
  induction l with
  | nil => rfl
  | cons a t ih =>
    simp only [List.flatMap_cons]
    rw [h a List.mem_cons_self, ih (fun x hx => h x (List.mem_cons_of_mem _ hx))]

theorem minBy_map {α β : Type} (le : β → β → Bool) (le' : α → α → Bool) (π : α → β)
    (l : List α) (h : ∀ a ∈ l, ∀ b ∈ l, le (π a) (π b) = le' a b) :
    minBy le (l.map π) = (minBy le' l).map π := by
  induction l with
  | nil => rfl
  | cons a t ih =>
    have ih' := ih (fun x hx y hy => h x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy))
    simp only [List.map_cons, minBy, ih']
    cases hm : minBy le' t with
    | none => rfl
    | some m =>
      have hmt : m ∈ t := minBy_mem hm
      simp only [Option.map_some]
      rw [h a List.mem_cons_self m (List.mem_cons_of_mem _ hmt)]
      split <;> rfl

theorem inj_of_nodup_map {α β : Type} {f : α → β} :
    ∀ {l : List α}, (l.map f).Nodup → ∀ {x y : α}, x ∈ l → y ∈ l → f x = f y → x = y
  | [], _, _, _, hx, _, _ => by cases hx
  | a :: t, hn, x, y, hx, hy, e => by
    rw [List.map_cons, List.nodup_cons] at hn
    rcases List.mem_cons.mp hx with rfl | hx' <;> rcases List.mem_cons.mp hy with rfl | hy'
    · rfl
    · exact absurd (e ▸ List.mem_map_of_mem hy') hn.1
    · exact absurd (e.symm ▸ List.mem_map_of_mem hx') hn.1
    · exact inj_of_nodup_map hn.2 hx' hy' e

variable {K : Type} [DecidableEq K] (P : Params K)

/-- A path relabelling `π` of the files of `G`: only paths change; every file
keeps the key it claims and the keys of its namespace parents; owner order
among claimants of one page is unchanged; paths stay unique. -/
structure Relabel (π : File → File) (G : Graph) : Prop where
  wf : WF G
  wf' : WF (G.map π)
  shape : ∀ f ∈ G, π f = { f with path := (π f).path }
  key : ∀ f ∈ G, P.key (P.name (π f)) = P.key (P.name f)
  names : ∀ f ∈ G, (P.fileNsNames (π f)).map P.key = (P.fileNsNames f).map P.key
  order : ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g → P.fileLe (π f) (π g) = P.fileLe f g

/-- The block-id image of a relabelling. -/
def relab (π : File → File) (G : Graph) (b : BlockId) : BlockId :=
  match fileAt G b.1 with
  | some f => ((π f).path, b.2)
  | none => b

variable {P} {π : File → File} {G : Graph}

omit [DecidableEq K] in
theorem Relabel.dir (H : Relabel P π G) {f : File} (hf : f ∈ G) : (π f).dir = f.dir :=
  by have h := congrArg File.dir (H.shape f hf); exact h
omit [DecidableEq K] in
theorem Relabel.format (H : Relabel P π G) {f : File} (hf : f ∈ G) : (π f).format = f.format :=
  by have h := congrArg File.format (H.shape f hf); exact h
omit [DecidableEq K] in
theorem Relabel.text (H : Relabel P π G) {f : File} (hf : f ∈ G) : (π f).text = f.text :=
  by have h := congrArg File.text (H.shape f hf); exact h

omit [DecidableEq K] in
theorem Relabel.kind (H : Relabel P π G) {f : File} (hf : f ∈ G) : kindOf (π f) = kindOf f := by
  unfold kindOf; rw [H.dir hf]

omit [DecidableEq K] in
theorem Relabel.claim (H : Relabel P π G) {f : File} (hf : f ∈ G) : P.claim (π f) = P.claim f := by
  unfold Params.claim; rw [H.kind hf, H.key f hf]

omit [DecidableEq K] in
theorem Relabel.inj (H : Relabel P π G) {f g : File} (hf : f ∈ G) (hg : g ∈ G)
    (e : π f = π g) : f = g := by
  have hn : (G.map (fun x => (π x).path)).Nodup := by
    have := H.wf'; unfold WF at this; rwa [List.map_map] at this
  exact inj_of_nodup_map hn hf hg (congrArg File.path e)

theorem Relabel.claimants (H : Relabel P π G) (p : Page K) :
    P.claimants (G.map π) p = (P.claimants G p).map π := by
  unfold Params.claimants
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro f hf
  simp only [Function.comp, H.claim hf]

theorem Relabel.owner (H : Relabel P π G) (p : Page K) :
    P.owner (G.map π) p = (P.owner G p).map π := by
  unfold Params.owner
  rw [H.claimants]
  apply minBy_map
  intro a ha b hb
  rw [P.mem_claimants] at ha hb
  exact H.order a ha.1 b hb.1 (ha.2.trans hb.2.symm)

theorem Relabel.isOwner (H : Relabel P π G) {f : File} (hf : f ∈ G) :
    P.isOwner (G.map π) (π f) = P.isOwner G f := by
  unfold Params.isOwner
  rw [H.claim hf, H.owner]
  apply Bool.eq_iff_iff.mpr
  simp only [decide_eq_true_eq]
  constructor
  · intro h
    cases ho : P.owner G (P.claim f) with
    | none => rw [ho] at h; cases h
    | some g =>
      rw [ho] at h
      have hg := (P.owner_spec ho).1
      simp only [Option.map_some, Option.some.injEq] at h
      rw [H.inj hg hf h]
  · intro h; rw [h]; rfl

theorem Relabel.real (H : Relabel P π G) (p : Page K) : P.real (G.map π) p = P.real G p := by
  unfold Params.real
  rw [List.any_map]
  apply Bool.eq_iff_iff.mpr
  simp only [List.any_eq_true, Function.comp, decide_eq_true_eq]
  constructor
  · rintro ⟨f, hf, e⟩; exact ⟨f, hf, (H.claim hf).symm.trans e⟩
  · rintro ⟨f, hf, e⟩; exact ⟨f, hf, (H.claim hf).trans e⟩

theorem Relabel.solid (H : Relabel P π G) (p : Page K) : P.solid (G.map π) p = P.solid G p := by
  unfold Params.solid
  rw [List.any_map]
  apply Bool.eq_iff_iff.mpr
  simp only [List.any_eq_true, Function.comp, Bool.and_eq_true, decide_eq_true_eq]
  have hc : ∀ f ∈ G, P.contentFree (π f) = P.contentFree f := by
    intro f hf
    unfold Params.contentFree
    rw [H.format hf, H.text hf]
  constructor
  · rintro ⟨f, hf, e, c⟩; exact ⟨f, hf, (H.claim hf).symm.trans e, by rw [← hc f hf]; exact c⟩
  · rintro ⟨f, hf, e, c⟩; exact ⟨f, hf, (H.claim hf).trans e, by rw [hc f hf]; exact c⟩

theorem Relabel.claimedKey (H : Relabel P π G) (k : K) :
    P.claimedKey (G.map π) k = P.claimedKey G k := by
  unfold Params.claimedKey; rw [H.real, H.real]

theorem Relabel.declaringOwners (H : Relabel P π G) :
    P.declaringOwners (G.map π) = (P.declaringOwners G).map π := by
  unfold Params.declaringOwners
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro f hf
  simp only [Function.comp, H.isOwner hf, H.kind hf]

theorem Relabel.aliasEdges (H : Relabel P π G) : P.aliasEdges (G.map π) = P.aliasEdges G := by
  unfold Params.aliasEdges
  rw [H.declaringOwners, flatMap_map2]
  apply flatMap_congr2
  intro f hf
  have hfG : f ∈ G := (List.mem_filter.mp hf).1
  rw [H.format hfG, H.text hfG, H.claim hfG]

theorem Relabel.declarers (H : Relabel P π G) (k : K) :
    P.declarers (G.map π) k = P.declarers G k := by
  unfold Params.declarers
  rw [H.declaringOwners, List.filter_map, List.map_map]
  have e : ∀ f ∈ P.declaringOwners G, (P.declaresKey ∘ π) f k = P.declaresKey f k := by
    intro f hf
    have hfG : f ∈ G := (List.mem_filter.mp hf).1
    simp only [Function.comp, Params.declaresKey, H.format hfG, H.text hfG]
  have e2 : (P.declaringOwners G).filter ((fun f => P.declaresKey f k) ∘ π) =
      (P.declaringOwners G).filter (fun f => P.declaresKey f k) :=
    List.filter_congr (fun f hf => e f hf)
  rw [e2]
  apply List.map_congr_left
  intro f hf
  exact H.claim (List.mem_filter.mp (List.mem_filter.mp hf).1).1

theorem Relabel.resolve (H : Relabel P π G) (s : String) :
    P.resolve (G.map π) s = P.resolve G s := by
  unfold Params.resolve Params.resolveKey
  rw [H.solid, H.solid, H.real, H.real, H.declarers]

omit [DecidableEq K] in
theorem Relabel.fileAt (H : Relabel P π G) {f : File} (hf : f ∈ G) :
    fileAt (G.map π) (π f).path = some (π f) :=
  fileAt_eq_of_mem H.wf' (List.mem_map_of_mem hf)

omit [DecidableEq K] in
theorem Relabel.occAt (H : Relabel P π G) {f : File} (hf : f ∈ G) (i : Nat) :
    P.occAt (G.map π) ((π f).path, i) = P.occAt G (f.path, i) := by
  unfold Params.occAt
  rw [H.fileAt hf, fileAt_eq_of_mem H.wf hf]
  simp only [H.format hf, H.text hf]

omit [DecidableEq K] in
theorem Relabel.refNames (H : Relabel P π G) {f : File} (hf : f ∈ G) (i : Nat) :
    P.refNames (G.map π) ((π f).path, i) = P.refNames G (f.path, i) := by
  unfold Params.refNames; rw [H.occAt hf]

omit [DecidableEq K] in
theorem Relabel.relab_eq (H : Relabel P π G) {f : File} (hf : f ∈ G) (i : Nat) :
    relab π G (f.path, i) = ((π f).path, i) := by
  unfold relab; rw [fileAt_eq_of_mem H.wf hf]

theorem mem_blocks_file {G : Graph} {b : BlockId} (hb : b ∈ P.blocks G) :
    ∃ f ∈ G, b = (f.path, b.2) := by
  unfold Params.blocks at hb
  rw [List.mem_flatMap] at hb
  obtain ⟨f, hf, hb⟩ := hb
  split at hb
  · rw [List.mem_map] at hb
    obtain ⟨i, _, e⟩ := hb
    exact ⟨f, hf, by rw [← e]⟩
  · cases hb

theorem Relabel.blocks (H : Relabel P π G) :
    P.blocks (G.map π) = (P.blocks G).map (relab π G) := by
  unfold Params.blocks
  rw [flatMap_map2, List.map_flatMap]
  apply flatMap_congr2
  intro f hf
  rw [H.isOwner hf, H.format hf, H.text hf]
  split
  · rw [List.map_map]
    apply List.map_congr_left
    intro i _
    simp only [Function.comp]
    rw [H.relab_eq hf]
  · rfl

/-- **Relabelling changes only block ids: references.** -/
theorem Relabel.references (H : Relabel P π G) {f : File} (hf : f ∈ G) (i : Nat) (t : Page K) :
    P.References (G.map π) ((π f).path, i) t ↔ P.References G (f.path, i) t := by
  have hR : P.resolve (G.map π) = P.resolve G := funext (H.resolve)
  simp only [Params.References, Params.refs, Params.concept, Params.aliasClass,
    H.refNames hf, H.aliasEdges, hR]

theorem Relabel.refKeys (H : Relabel P π G) : P.refKeys (G.map π) = P.refKeys G := by
  unfold Params.refKeys
  rw [H.blocks, flatMap_map2]
  apply flatMap_congr2
  intro b hb
  obtain ⟨f, hf, e⟩ := mem_blocks_file hb
  rw [e, H.relab_eq hf, H.refNames hf]

omit [DecidableEq K] in
theorem Relabel.nameKeys (H : Relabel P π G) : P.nameKeys (G.map π) = P.nameKeys G := by
  unfold Params.nameKeys
  rw [flatMap_map2]
  exact flatMap_congr2 (fun f hf => H.names f hf)

/-- **Relabelling changes only block ids: the page universe is the same list.** -/
theorem Relabel.pagesR (H : Relabel P π G) (r : Bool) : P.pagesR r (G.map π) = P.pagesR r G := by
  have hd : ∀ k, P.declaredKey (G.map π) k = P.declaredKey G k := fun k => by
    unfold Params.declaredKey; rw [H.aliasEdges]
  have hrp : P.realPages (G.map π) = P.realPages G := by
    unfold Params.realPages; rw [List.map_map]
    exact List.map_congr_left (fun f hf => H.claim hf)
  unfold Params.pagesR Params.refOnlyPagesR Params.pageSeedKeys
  rw [hrp, H.refKeys, H.nameKeys]
  simp only [H.claimedKey, hd]

/-- **Relabelling changes only block ids: every page keeps its blocks.** -/
theorem Relabel.pageBlocks (H : Relabel P π G) (p : Page K) :
    P.pageBlocks (G.map π) p = (P.pageBlocks G p).map (relab π G) := by
  unfold Params.pageBlocks
  rw [H.owner]
  cases ho : P.owner G p with
  | none => rfl
  | some f =>
    have hf := (P.owner_spec ho).1
    simp only [Option.map_some, List.map_map]
    rw [H.format hf, H.text hf]
    apply List.map_congr_left
    intro i _
    simp only [Function.comp]
    rw [H.relab_eq hf]

/-! ## Same-key renames that move files -/

/-- The rewriter with paths frozen: the T3 half of the factorisation. -/
def keepPath (R : CM.Rewriter) : CM.Rewriter :=
  ⟨R.covers, fun m f => { R.apply m f with path := f.path }⟩

/-- The relabelling half: the file at `g`'s path is replaced by its real image. -/
def moveOf (R : CM.Rewriter) (m : String → String) (G : Graph) (g : File) : File :=
  match fileAt G g.path with
  | some f => R.apply m f
  | none => g

theorem renameBy_factor {R : CM.Rewriter} {m : String → String} (hw : WF G) :
    (renameBy (keepPath R) m G).map (moveOf R m G) = renameBy R m G := by
  unfold renameBy
  rw [List.map_map]
  apply List.map_congr_left
  intro f hf
  simp only [Function.comp, moveOf, keepPath]
  rw [fileAt_eq_of_mem hw hf]

theorem wf_keepPath {R : CM.Rewriter} {m : String → String} (hw : WF G) :
    WF (renameBy (keepPath R) m G) := by
  unfold WF renameBy at *
  rw [List.map_map]
  have : (File.path ∘ (keepPath R).apply m) = File.path := rfl
  rw [this]; exact hw

theorem moveOf_keepPath {R : CM.Rewriter} {m : String → String} (hw : WF G) {f : File}
    (hf : f ∈ G) : moveOf R m G ((keepPath R).apply m f) = R.apply m f := by
  simp only [moveOf, keepPath]
  rw [fileAt_eq_of_mem hw hf]

variable (P) in
/-- **CM-MOVE (σ = id transport, files may move).** If the path-frozen
rewriter satisfies T3's hypotheses with σ = id, and the real rewriter keeps
every file's claimed key, namespace-parent keys and owner order, then the page
universe is unchanged, every block keeps exactly the pages it references at its
image id (`blockMap`, i.e. at the new path), and every page keeps its blocks.
No path law is needed for the real rewriter. -/
theorem moving_transport {R : CM.Rewriter} {m : String → String}
    (H1 : P.RenameHyps (keepPath R) m id G) (hw' : WF (renameBy R m G))
    (hKn : ∀ f ∈ G, (P.fileNsNames ((keepPath R).apply m f)).map P.key = (P.fileNsNames f).map P.key)
    (hRk : ∀ f ∈ G, P.key (P.name (R.apply m f)) = P.key (P.name f))
    (hRn : ∀ f ∈ G, (P.fileNsNames (R.apply m f)).map P.key = (P.fileNsNames f).map P.key)
    (hRo : ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g →
      P.fileLe (R.apply m f) (R.apply m g) = P.fileLe f g) :
    (∀ (r : Bool) (p : Page K), p ∈ P.pagesR r (renameBy R m G) ↔ p ∈ P.pagesR r G) ∧
    (∀ b ∈ P.blocks G, ∀ t : Page K, P.InPlay G t.2 →
      (P.References G b t ↔ P.References (renameBy R m G) (blockMap R m G b) t)) ∧
    (∀ p : Page K, P.InPlay G p.2 →
      P.pageBlocks (renameBy R m G) p = (P.pageBlocks G p).map (blockMap R m G)) := by
  have hw := H1.wf
  have hw₁ := H1.wf'
  have hK := H1.laws
  have hx : P.GraphExceptionFree (keepPath R) id G := fun _ _ _ _ _ _ _ => rfl
  -- T3 on the path-frozen rewriter
  have Tp : ∀ (r : Bool) (p : Page K),
      p ∈ P.pagesR r (renameBy (keepPath R) m G) ↔ p ∈ P.pagesR r G := fun r p =>
    ⟨fun hp => by obtain ⟨q, hq, e⟩ := pagesR_rename_onto H1 hx hp; rw [← e]; exact hq,
     fun hp => pagesR_rename_fwd H1 hx hp⟩
  have Tr : ∀ b ∈ P.blocks G, ∀ t : Page K, P.InPlay G t.2 →
      (P.References G b t ↔ P.References (renameBy (keepPath R) m G)
        (blockMap (keepPath R) m G b) t) :=
    fun b hb t ht => references_rename H1 hb (fun _ _ _ _ _ => rfl) ht
  have Tb : ∀ p : Page K, P.InPlay G p.2 →
      P.pageBlocks (renameBy (keepPath R) m G) p =
        (P.pageBlocks G p).map (blockMap (keepPath R) m G) :=
    fun p hp => pageBlocks_rename H1 hp
  -- the relabelling
  have Hk : ∀ f ∈ G, P.key (P.name ((keepPath R).apply m f)) = P.key (P.name f) :=
    fun f hf => hK.name f hf
  have Hc : ∀ f ∈ G, P.claim ((keepPath R).apply m f) = P.claim f := fun f hf => by
    unfold Params.claim; rw [Hk f hf]
    have : kindOf ((keepPath R).apply m f) = kindOf f := by
      unfold kindOf; rw [hK.dir f hf]
    rw [this]
  have H : Relabel P (moveOf R m G) (renameBy (keepPath R) m G) := by
    refine ⟨hw₁, by rw [renameBy_factor hw]; exact hw', ?_, ?_, ?_, ?_⟩
    · intro g hg
      obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hg
      rw [moveOf_keepPath hw hf]
      simp only [keepPath]
    · intro g hg
      obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hg
      rw [moveOf_keepPath hw hf, hRk f hf, Hk f hf]
    · intro g hg
      obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hg
      rw [moveOf_keepPath hw hf, hRn f hf, hKn f hf]
    · intro g hg g' hg' hc
      obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hg
      obtain ⟨f', hf', rfl⟩ := List.mem_map.mp hg'
      rw [moveOf_keepPath hw hf, moveOf_keepPath hw hf', hK.order f hf f' hf' (by
        rw [← Hc f hf, ← Hc f' hf']; exact hc)]
      exact hRo f hf f' hf' (by rw [← Hc f hf, ← Hc f' hf']; exact hc)
  have hfac := renameBy_factor (R := R) (m := m) hw
  -- block ids
  have hbm : ∀ f ∈ G, ∀ i, relab (moveOf R m G) (renameBy (keepPath R) m G)
      (blockMap (keepPath R) m G (f.path, i)) = blockMap R m G (f.path, i) := by
    intro f hf i
    rw [blockMap_eq hw hf, blockMap_eq hw hf]
    have hmem : (keepPath R).apply m f ∈ renameBy (keepPath R) m G := List.mem_map_of_mem hf
    rw [H.relab_eq hmem, moveOf_keepPath hw hf]
  refine ⟨fun r p => ?_, fun b hb t ht => ?_, fun p hp => ?_⟩
  · rw [← hfac, H.pagesR]; exact Tp r p
  · obtain ⟨f, hf, e⟩ := mem_blocks_file hb
    rw [Tr b hb t ht, e, ← hbm f hf, ← hfac, blockMap_eq hw hf]
    have hmem : (keepPath R).apply m f ∈ renameBy (keepPath R) m G := List.mem_map_of_mem hf
    rw [H.relab_eq hmem, moveOf_keepPath hw hf, ← moveOf_keepPath hw hf (R := R) (m := m)]
    rw [H.references hmem]
  · rw [← hfac, H.pageBlocks, Tb p hp, List.map_map]
    apply List.map_congr_left
    intro b hb
    have hbG : b ∈ P.blocks G := P.pageBlocks_sub_blocks hb
    obtain ⟨f, hf, e⟩ := mem_blocks_file hbG
    simp only [Function.comp]
    rw [e, hbm f hf]

variable (P) in
/-- **CM-CASE-MOVE (same-key transport, files may move).** The same-key
specialisation: under the same-key laws, T3's key hypotheses for the frozen
rewriter follow from its namespace-parent agreement alone. -/
theorem same_key_moving_transport {R : CM.Rewriter} {old new : String}
    (L : CaseRename7.SameKeyLaws P old new)
    (hw : WF G) (hw' : WF (renameBy R (nsSubst P.key P.urlHasOrigin old new) G))
    (hK : P.RwLaws (keepPath R) (nsSubst P.key P.urlHasOrigin old new) id G)
    (hKn : ∀ f ∈ G, (P.fileNsNames ((keepPath R).apply (nsSubst P.key P.urlHasOrigin old new) f)).map P.key =
      (P.fileNsNames f).map P.key)
    (hRk : ∀ f ∈ G, P.key (P.name (R.apply (nsSubst P.key P.urlHasOrigin old new) f)) = P.key (P.name f))
    (hRn : ∀ f ∈ G, (P.fileNsNames (R.apply (nsSubst P.key P.urlHasOrigin old new) f)).map P.key =
      (P.fileNsNames f).map P.key)
    (hRo : ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g →
      P.fileLe (R.apply (nsSubst P.key P.urlHasOrigin old new) f) (R.apply (nsSubst P.key P.urlHasOrigin old new) g) = P.fileLe f g) :
    (∀ (r : Bool) (p : Page K),
      p ∈ P.pagesR r (renameBy R (nsSubst P.key P.urlHasOrigin old new) G) ↔ p ∈ P.pagesR r G) ∧
    (∀ b ∈ P.blocks G, ∀ t : Page K, P.InPlay G t.2 →
      (P.References G b t ↔
        P.References (renameBy R (nsSubst P.key P.urlHasOrigin old new) G)
          (blockMap R (nsSubst P.key P.urlHasOrigin old new) G b) t)) ∧
    (∀ p : Page K, P.InPlay G p.2 →
      P.pageBlocks (renameBy R (nsSubst P.key P.urlHasOrigin old new) G) p =
        (P.pageBlocks G p).map (blockMap R (nsSubst P.key P.urlHasOrigin old new) G)) :=
  moving_transport P ⟨hw, wf_keepPath hw, hK, CaseRename7.same_key_keyHyps P L hKn⟩ hw' hKn hRk hRn hRo

end CaseRelabel7
end CM
