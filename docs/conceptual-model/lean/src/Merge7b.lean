/- Round 7b (verifier r7 findings 1 and 4): the survivor's BODY and STORED-QUERY
laws, and what they give.

Tine's survivor (`pages.rs` `merged_survivor`, `pages/merge_payload.rs`
`payload`) is: the survivor's page header (pre-block; the target's, with the
source's header properties joined in) ++ the target's body blocks, renamed ++
at most one moved block (the source's leftover header lines) ++ the source's
body blocks, renamed. Header handling may combine lines; body blocks are kept
one by one, in order, with their rewritten text (journal 2026-10-05 P2: always
append every source block). `BodyLaw` states exactly that layout, so a survivor
that merges or drops body blocks violates it (`MergeWitness7.collapse_rejected`).

`QueryLaw` is the survivor's stored-query contract, the merge analogue of the
round-6 CMQ law `QueryRename6.FaithfulOn.stored`: the queries stored in every
survivor block are its origin blocks' queries with the page-name map applied.

The occurrence-level results of `Merge7` (`MergeOpHyps`) stay separate lemmas;
`MergeHyps7b` extends their hypotheses. -/
import Merge7

set_option linter.unusedSectionVars false
namespace CM
namespace Merge7b
open Params Merge7 QueryRename6

variable {K L : Type} [DecidableEq K] (P : Params K) (Q : QParams K L)

/-- Header blocks of a file: its pre-block, if it has one (`QParams.preBlock`,
read as in `preBlockOf`). -/
def hdr (f : File) : Nat :=
  if Q.preBlock f.format f.text && decide (0 < (P.outline f.format f.text).length) then 1 else 0

theorem hdr_le_one (f : File) : hdr P Q f ≤ 1 := by unfold hdr; split <;> omega

/-- A file's blocks after the rename rewrite. -/
def rwOut (R : CM.Rewriter) (m : String → String) (f : File) : List String :=
  P.outline (R.apply m f).format (R.apply m f).text

/-- The survivor's blocks. -/
def survOut (J : Merger) (m : String → String) (S D : File) : List String :=
  P.outline (J.apply m S D).format (J.apply m S D).text

/-- The survivor layout with `p` header blocks and `q` moved blocks. -/
def BodyAt (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) (p q : Nat) : Prop :=
  (survOut P J m S D).length =
      p + ((rwOut P R m D).drop (hdr P Q D)).length + q + ((rwOut P R m S).drop (hdr P Q S)).length ∧
  ((survOut P J m S D).drop p).take ((rwOut P R m D).drop (hdr P Q D)).length =
      (rwOut P R m D).drop (hdr P Q D) ∧
  (survOut P J m S D).drop (p + ((rwOut P R m D).drop (hdr P Q D)).length + q) =
      (rwOut P R m S).drop (hdr P Q S) ∧
  (∀ j, j < ((rwOut P R m D).drop (hdr P Q D)).length →
      J.origin m S D (p + j) = [(false, hdr P Q D + j)]) ∧
  (∀ j, j < ((rwOut P R m S).drop (hdr P Q S)).length →
      J.origin m S D (p + ((rwOut P R m D).drop (hdr P Q D)).length + q + j) = [(true, hdr P Q S + j)])

instance (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) (p q : Nat) :
    Decidable (BodyAt P Q R J m S D p q) := by unfold BodyAt; infer_instance

/-- **Body law.** At most one header block and at most one moved block; every
target body block, then every source body block, in order, each its own
survivor block with its rewritten text and that single origin. -/
def BodyLaw (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) : Prop :=
  ∃ p, p < 2 ∧ ∃ q, q < 2 ∧ BodyAt P Q R J m S D p q

instance (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) :
    Decidable (BodyLaw P Q R J m S D) := by unfold BodyLaw; infer_instance

/-- The stored queries of an old source (`true`) or target (`false`) block. -/
def originQueries (SP : StoredParser L) (S D : File) : Origin → List (Query L)
  | (true, j) => (((P.outline S.format S.text)[j]?).map (SP.extract S.format)).getD []
  | (false, j) => (((P.outline D.format D.text)[j]?).map (SP.extract D.format)).getD []

/-- **Survivor stored-query law** (the merge analogue of the CMQ law): the
queries stored in survivor block `i` are its origins' queries, renamed. -/
def QueryLaw (SP : StoredParser L) (J : Merger) (m : String → String) (S D : File) : Prop :=
  ∀ i, i < (survOut P J m S D).length →
    (((survOut P J m S D)[i]?).map (SP.extract (J.apply m S D).format)).getD [] =
      (J.origin m S D i).flatMap (fun x => (originQueries P SP S D x).map (renameQuery m))

instance [DecidableEq L] (SP : StoredParser L) (J : Merger) (m : String → String) (S D : File) :
    Decidable (QueryLaw P SP J m S D) := by unfold QueryLaw; infer_instance

/-- The round-7b merge hypotheses: round 7's, the rename laws also on the
source and the target (their texts are rewritten like every other file's; this
includes the decided `alias::` member rewrite), equal formats (Tine refuses
otherwise), the body law, the survivor query law, and the CMQ law on the other
files. -/
structure MergeHyps7b (SP : StoredParser L) (R : CM.Rewriter) (J : Merger) (m : String → String)
    (σ : K → K) (S D : File) (G : Graph) : Prop extends MergeOpHyps P R J m σ S D G where
  lawsSD : P.RwLaws R m σ [S, D]
  sameFormat : S.format = D.format
  body : BodyLaw P Q R J m S D
  queries : QueryLaw P SP J m S D
  storedOthers : ∀ f ∈ others S D G,
    (P.outline (R.apply m f).format (R.apply m f).text).map (SP.extract (R.apply m f).format) =
      ((P.outline f.format f.text).map (SP.extract f.format)).map (List.map (renameQuery m))

/-- Round 8: the body and stored-query laws over `MergeBase` (survivor of either
kind, M-Q7), with the source/target rewrite laws read under a key map `σ` that
may differ from the merge's key map `τ`. `τ = σ` for a merge into a page or a
journal claiming `new`; for a merge into the owner of the alias `new` (M-Q5a)
`τ` sends the source key to the owner while the rewriter sends the source's
name to `new` (`Merge8`). Round 8b: the alias contracts of the extended
`MergeBase` read under the text map `σ` as well (its `σa`), since the rewritten
text spells every `alias:: src` as `alias:: new`. -/
structure MergeBodyHyps (SP : StoredParser L) (R : CM.Rewriter) (J : Merger) (m : String → String)
    (σ τ : K → K) (S D : File) (G : Graph) : Prop extends MergeBase P R J m τ S D G σ where
  lawsSD : P.RwLaws R m σ [S, D]
  sameFormat : S.format = D.format
  body : BodyLaw P Q R J m S D
  queries : QueryLaw P SP J m S D
  storedOthers : ∀ f ∈ others S D G,
    (P.outline (R.apply m f).format (R.apply m f).text).map (SP.extract (R.apply m f).format) =
      ((P.outline f.format f.text).map (SP.extract f.format)).map (List.map (renameQuery m))

variable {P Q} {SP : StoredParser L} {R : CM.Rewriter} {J : Merger} {m : String → String}
  {σ τ : K → K} {S D : File} {G : Graph}

/-- Every round-7b instance is a `MergeBodyHyps` instance with `τ = σ`. -/
theorem MergeHyps7b.toBody (H : MergeHyps7b P Q SP R J m σ S D G) :
    MergeBodyHyps P Q SP R J m σ σ S D G where
  toMergeBase := H.toMergeOpHyps.toMergeBase
  lawsSD := H.lawsSD
  sameFormat := H.sameFormat
  body := H.body
  queries := H.queries
  storedOthers := H.storedOthers

/-! ## Body blocks: every one kept, separately, in order, with its text -/

/-- An ordinary (non-header) block of the target (`false`) or source (`true`). -/
def IsBody (P : Params K) (Q : QParams K L) (S D : File) : Origin → Prop
  | (false, j) => hdr P Q D ≤ j ∧ j < (P.outline D.format D.text).length
  | (true, j) => hdr P Q S ≤ j ∧ j < (P.outline S.format S.text).length

/-- Append order: the target's body, then the source's, each in its own order. -/
def OLt : Origin → Origin → Prop
  | (false, i), (false, j) => i < j
  | (false, _), (true, _) => True
  | (true, i), (true, j) => i < j
  | (true, _), (false, _) => False

/-- The old file of an origin. -/
def oldFile (S D : File) : Origin → File
  | (true, _) => S
  | (false, _) => D

theorem rwOut_length (H : MergeBodyHyps P Q SP R J m σ τ S D G) {f : File} (hf : f = S ∨ f = D) :
    (rwOut P R m f).length = (P.outline f.format f.text).length :=
  outline_length_rename H.lawsSD (by rcases hf with rfl | rfl <;> simp)

/-- **MG4' (no block lost, strong form).** There is an index map `ι` from the
old body blocks of the target and the source into the survivor that is strictly
increasing in append order (so injective: no two body blocks share a survivor
block), and each body block's image is a block of the after-graph, has exactly
that block as origin, is its image under `Img`, and has its rewritten text. -/
theorem bodies_kept (H : MergeBodyHyps P Q SP R J m σ τ S D G) :
    ∃ ι : Origin → Nat,
      (∀ x y, IsBody P Q S D x → IsBody P Q S D y → OLt x y → ι x < ι y) ∧
      ∀ x, IsBody P Q S D x →
        (D.path, ι x) ∈ P.blocks (mergeBy R J m S D G) ∧
        J.origin m S D (ι x) = [x] ∧
        Img R J m S D G ((oldFile S D x).path, x.2) (D.path, ι x) ∧
        (survOut P J m S D)[ι x]? = (rwOut P R m (oldFile S D x))[x.2]? := by
  obtain ⟨p, _, q, _, hlen, htake, hdrop, horD, horS⟩ := H.body
  have lD := rwOut_length H (f := D) (Or.inr rfl)
  have lS := rwOut_length H (f := S) (Or.inl rfl)
  simp only [List.length_drop] at hlen htake hdrop horD horS
  let tb := (rwOut P R m D).length - hdr P Q D
  let ι : Origin → Nat := fun x => match x with
    | (false, j) => p + (j - hdr P Q D)
    | (true, j) => p + tb + q + (j - hdr P Q S)
  refine ⟨ι, ?_, ?_⟩
  · rintro ⟨a, i⟩ ⟨b, j⟩ hx hy hlt
    cases a <;> cases b <;> simp only [IsBody, OLt] at hx hy hlt <;> simp only [ι, tb] <;> omega
  · rintro ⟨c, j⟩ hx
    have hmem : ∀ i, i < (survOut P J m S D).length → (D.path, i) ∈ P.blocks (mergeBy R J m S D G) :=
      fun i hi => (mem_blocks_merge7 H.toMergeBase _).mpr (Or.inl ⟨rfl, hi⟩)
    cases c
    · simp only [IsBody] at hx
      have ho := horD (j - hdr P Q D) (by omega)
      have ej : hdr P Q D + (j - hdr P Q D) = j := by omega
      rw [ej] at ho
      refine ⟨hmem _ (by simp only [ι, tb]; omega), ho, Or.inr (Or.inl ⟨rfl, rfl, by
        simp only [ι]; rw [ho]; exact List.mem_singleton_self _⟩), ?_⟩
      have h := congrArg (fun l => l[j - hdr P Q D]?) htake
      rw [List.getElem?_take_of_lt (by omega), List.getElem?_drop, List.getElem?_drop, ej] at h
      exact h
    · simp only [IsBody] at hx
      have ho := horS (j - hdr P Q S) (by omega)
      have ej : hdr P Q S + (j - hdr P Q S) = j := by omega
      rw [ej] at ho
      refine ⟨hmem _ (by simp only [ι, tb]; omega), ho, Or.inl ⟨rfl, rfl, by
        simp only [ι, tb]; rw [ho]; exact List.mem_singleton_self _⟩, ?_⟩
      have h := congrArg (fun l => l[j - hdr P Q S]?) hdrop
      rw [List.getElem?_drop, List.getElem?_drop, ej] at h
      exact h

/-! ## Stored queries: exact syntax -/

/-- **MQ1 (survivor blocks).** The queries stored in every survivor block are its
origins' stored queries, renamed. -/
theorem storedAt_survivor (H : MergeBodyHyps P Q SP R J m σ τ S D G) {i : Nat}
    (hi : i < (survOut P J m S D).length) :
    storedAt P SP (mergeBy R J m S D G) (D.path, i) =
      (J.origin m S D i).flatMap (fun x =>
        (storedAt P SP G ((oldFile S D x).path, x.2)).map (renameQuery m)) := by
  have h := storedAt_some P SP H.wf' (survivor_mem H.toMergeBase) i
  rw [H.merger.path] at h
  rw [h]
  change (((survOut P J m S D)[i]?).map (SP.extract (J.apply m S D).format)).getD [] = _
  rw [H.queries i hi]
  congr 1
  funext x
  obtain ⟨c, j⟩ := x
  cases c
  · simp only [oldFile, originQueries]; rw [storedAt_some P SP H.wf H.dst]
  · simp only [oldFile, originQueries]; rw [storedAt_some P SP H.wf H.src]

/-- **MQ1 (other blocks).** As the CMQ law: renamed, block by block. -/
theorem storedAt_other (H : MergeBodyHyps P Q SP R J m σ τ S D G) {f : File}
    (hf : f ∈ others S D G) (i : Nat) :
    storedAt P SP (mergeBy R J m S D G) ((R.apply m f).path, i) =
      (storedAt P SP G (f.path, i)).map (renameQuery m) := by
  rw [storedAt_some P SP H.wf' (other_mem hf), storedAt_some P SP H.wf (others_sub hf)]
  have h := congrArg (fun xs => xs[i]?) (H.storedOthers f hf)
  simp only [List.getElem?_map] at h
  rw [h]
  cases (P.outline f.format f.text)[i]? <;> rfl

/-- **MQ1 (body blocks).** A body block's stored queries reach its own survivor
block, renamed, and nothing else is stored there. -/
theorem storedAt_body (H : MergeBodyHyps P Q SP R J m σ τ S D G) {x : Origin} {i : Nat}
    (hi : i < (survOut P J m S D).length) (ho : J.origin m S D i = [x]) :
    storedAt P SP (mergeBy R J m S D G) (D.path, i) =
      (storedAt P SP G ((oldFile S D x).path, x.2)).map (renameQuery m) := by
  rw [storedAt_survivor H hi, ho, List.flatMap_singleton]

/-! ## Negation-free queries go forward (verifier r7 finding 4)

What the stored-query law gives. A query without `not` that matched an old
block (or page) matches, after its arguments are renamed, the block's image,
as long as the image keeps the block's query inputs (`BlockForward`). This is
forward inclusion only: an image block may match more (it gained the other
page's references, or a new ancestor), and `not` breaks even inclusion
(`MergeWitness7.mw_queries`). -/

/-- No `not` anywhere. -/
def Positive {L : Type} : Query L → Prop
  | .leaf _ => True
  | .and q r => Positive q ∧ Positive r
  | .or q r => Positive q ∧ Positive r
  | .not _ => False

/-- The query inputs of block `b` of `G` reach block `b'` of `G'`: every path
reference key, mapped by `σ`, and the page, mapped by `σ`. -/
structure BlockForward (P : Params K) (Q : QParams K L) (σ : K → K) (G G' : Graph)
    (b b' : BlockId) : Prop where
  keys : ∀ k ∈ P.pathRefKeys Q G b, σ k ∈ P.pathRefKeys Q G' b'
  page : P.pageOfBlock G' b' = (P.pageOfBlock G b).map (σp σ)

/-- Alias edges go forward under `σ`. -/
def EdgesForward (P : Params K) (σ : K → K) (G G' : Graph) : Prop :=
  ∀ e ∈ P.aliasEdges G, (σ e.1, σ e.2) ∈ P.aliasEdges G'

theorem cls_forward {G G' : Graph} (hE : EdgesForward P σ G G') {a b : K}
    (h : cls (P.aliasEdges G) a b) : cls (P.aliasEdges G') (σ a) (σ b) :=
  Reach.mono (fun e he => by
    obtain ⟨e0, he0, rfl⟩ := List.mem_map.mp he
    exact hE e0 he0) (Reach.map_fwd σ h)

theorem leafPage_fwd {G G' : Graph} (l : Leaf L) (hl : leafDefined l)
    (ha : ∀ s ∈ leafArgs l, P.key (m s) = σ (P.key s)) {p : Page K}
    (h : P.leafPage Q G l p = true) : P.leafPage Q G' (renameLeaf m l) (σp σ p) = true := by
  induction l with
  | refs => simp [leafPage] at h
  | name s =>
    simp only [renameLeaf, leafPage, σp, decide_eq_true_eq] at h ⊢
    rw [ha s (by simp [leafArgs]), h]
  | page l ih =>
    simp only [renameLeaf, leafPage] at h ⊢
    exact ih hl (fun s hs => ha s (by simpa [leafArgs] using hs)) h
  | given => exact hl.elim

theorem leafBlock_fwd {G G' : Graph} (hE : EdgesForward P σ G G') {b b' : BlockId}
    (hB : BlockForward P Q σ G G' b b') (l : Leaf L) (hl : leafDefined l)
    (ha : ∀ s ∈ leafArgs l, P.key (m s) = σ (P.key s))
    (h : P.leafBlock3 Q G l b = true) : P.leafBlock3 Q G' (renameLeaf m l) b' = true := by
  cases l with
  | refs s =>
    simp only [renameLeaf, leafBlock3, List.any_eq_true, sameCls_iff] at h ⊢
    obtain ⟨k, hk, hc⟩ := h
    refine ⟨σ k, hB.keys k hk, ?_⟩
    rw [ha s (by simp [leafArgs])]
    exact cls_forward hE hc
  | name => simp [leafBlock3] at h
  | page l =>
    simp only [renameLeaf, leafBlock3, hB.page] at h ⊢
    cases hp : P.pageOfBlock G b with
    | none => rw [hp] at h; cases h
    | some p =>
      rw [hp] at h
      exact leafPage_fwd l hl (fun s hs => ha s (by simpa [leafArgs] using hs)) h
  | given => exact hl.elim

theorem bval_fwd (q : Query L) (hq : Positive q) {v w : Leaf L → Bool}
    (hl : ∀ l ∈ q.leaves, v l = true → w (renameLeaf m l) = true)
    (h : q.bval v = true) : (renameQuery m q).bval w = true := by
  induction q with
  | leaf l => exact hl l (by simp [Query.leaves]) h
  | and q r ihq ihr =>
    simp only [renameQuery, Query.bval, Bool.and_eq_true] at h ⊢
    exact ⟨ihq hq.1 (fun l hm => hl l (by simp [Query.leaves, hm])) h.1,
      ihr hq.2 (fun l hm => hl l (by simp [Query.leaves, hm])) h.2⟩
  | or q r ihq ihr =>
    simp only [renameQuery, Query.bval, Bool.or_eq_true] at h ⊢
    rcases h with h | h
    · exact Or.inl (ihq hq.1 (fun l hm => hl l (by simp [Query.leaves, hm])) h)
    · exact Or.inr (ihr hq.2 (fun l hm => hl l (by simp [Query.leaves, hm])) h)
  | not => exact hq.elim

/-- **MQ2 (blocks).** A negation-free query without `given` leaves whose
arguments are renamed consistently with `σ` keeps every block it matched, at
the block's image. -/
theorem block_forward {G G' : Graph} (hE : EdgesForward P σ G G') {b b' : BlockId}
    (hB : BlockForward P Q σ G G' b b') (q : Query L) (hq : Positive q) (hd : Defined q)
    (ha : ∀ s ∈ queryArgs q, P.key (m s) = σ (P.key s))
    (h : P.evalNode3 Q G q (.block b) = true) :
    P.evalNode3 Q G' (renameQuery m q) (.block b') = true :=
  bval_fwd q hq (fun l hm hv => leafBlock_fwd hE hB l (hd l hm)
    (fun s hs => ha s (List.mem_flatMap.mpr ⟨l, hm, hs⟩)) hv) h

/-- **MQ2 (pages).** The same on page anchors: a page goes to its σ-image. -/
theorem page_forward {G G' : Graph} {p : Page K} (q : Query L) (hq : Positive q)
    (hd : Defined q) (ha : ∀ s ∈ queryArgs q, P.key (m s) = σ (P.key s))
    (h : P.evalNode3 Q G q (.page p) = true) :
    P.evalNode3 Q G' (renameQuery m q) (.page (σp σ p)) = true :=
  bval_fwd q hq (fun l hm hv => leafPage_fwd l (hd l hm)
    (fun s hs => ha s (List.mem_flatMap.mpr ⟨l, hm, hs⟩)) hv) h

/-! ### When a merged graph keeps a block's query inputs -/

theorem edges_forward_merge7 (H : MergeBase P R J m σ S D G) :
    EdgesForward P σ G (mergeBy R J m S D G) :=
  fun e he => (aliasEdges_merge7 H _).mpr (List.mem_map.mpr ⟨e, he, rfl⟩)

theorem refKey_fwd {G G' : Graph} {a a' : BlockId}
    (hocc : P.occAt G' a' = (P.occAt G a).map (rwOcc R.covers m))
    (hns : NsForward P R m σ (P.occAt G a)) {k : K} (hk : k ∈ (P.refNames G a).map P.key) :
    σ k ∈ (P.refNames G' a').map P.key := by
  obtain ⟨u, hu, rfl⟩ := List.mem_map.mp hk
  obtain ⟨o, ho, hu⟩ := P.mem_refNames.mp hu
  obtain ⟨v, hv, he⟩ := hns o ho u hu
  exact List.mem_map.mpr ⟨v, P.mem_refNames.mpr ⟨rwOcc R.covers m o,
    by rw [hocc]; exact List.mem_map_of_mem ho, hv⟩, he⟩

/-- A block keeps its query inputs when its occurrences are rewritten by
`rwOcc` under the forward namespace condition, its old ancestors' keys still
reach the image, and its page goes to the σ-image. -/
theorem blockForward_of {G G' : Graph} {b b' : BlockId}
    (hocc : P.occAt G' b' = (P.occAt G b).map (rwOcc R.covers m))
    (hns : NsForward P R m σ (P.occAt G b))
    (hanc : ∀ a ∈ ancestors Q G b, ∀ k ∈ (P.refNames G a).map P.key,
      σ k ∈ P.pathRefKeys Q G' b')
    (hpage : P.pageOfBlock G' b' = (P.pageOfBlock G b).map (σp σ)) :
    BlockForward P Q σ G G' b b' := by
  refine ⟨fun k hk => ?_, hpage⟩
  simp only [pathRefKeys, List.mem_append, List.mem_flatMap] at hk
  rcases hk with ((hk | ⟨a, ha, hk⟩) | hk)
  · simp only [pathRefKeys, List.mem_append]
    exact Or.inl (Or.inl (refKey_fwd hocc hns hk))
  · exact hanc a ha k hk
  · simp only [pathRefKeys, List.mem_append, hpage]
    apply Or.inr
    cases hp : P.pageOfBlock G b with
    | none => simp [hp] at hk
    | some p =>
      simp only [hp, Option.map_some, Option.toList_some, List.mem_singleton] at hk ⊢
      rw [hk]; rfl

/-- Ancestors of a rewritten other file, given that its rewrite keeps the
outline depths. -/
theorem ancestors_other (H : MergeBase P R J m σ S D G) {f : File} (hf : f ∈ others S D G)
    (hd : Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text) (i : Nat) :
    ancestors Q (mergeBy R J m S D G) ((R.apply m f).path, i) =
      (ancestors Q G (f.path, i)).map (fun a => ((R.apply m f).path, a.2)) := by
  simp only [ancestors, fileAt_eq_of_mem H.wf' (other_mem hf),
    fileAt_eq_of_mem H.wf (others_sub hf), hd, List.map_map]
  rfl

/-- **MQ3 (other blocks).** A block of any file other than the source and the
target keeps its query inputs, given the depth law for its file and the
forward namespace condition on its blocks. -/
theorem blockForward_other (H : MergeBase P R J m σ S D G) {f : File} (hf : f ∈ others S D G)
    (hd : Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    (hns : ∀ j, NsForward P R m σ (P.occAt G (f.path, j))) (i : Nat) :
    BlockForward P Q σ G (mergeBy R J m S D G) (f.path, i) ((R.apply m f).path, i) := by
  apply blockForward_of (occAt_other H hf i) (hns i)
  · intro a ha k hk
    obtain ⟨j, _, rfl⟩ := List.mem_map.mp ha
    simp only [pathRefKeys, List.mem_append, List.mem_flatMap]
    refine Or.inl (Or.inr ⟨((R.apply m f).path, j), ?_, refKey_fwd (occAt_other H hf j) (hns j) hk⟩)
    rw [ancestors_other H hf hd]
    exact List.mem_map.mpr ⟨(f.path, j), ha, rfl⟩
  · simp [pageOfBlock, fileAt_eq_of_mem H.wf' (other_mem hf),
      fileAt_eq_of_mem H.wf (others_sub hf), claim_apply H.laws hf]

theorem oldFile_cases (S D : File) (x : Origin) : oldFile S D x = S ∨ oldFile S D x = D := by
  obtain ⟨c, j⟩ := x; cases c
  · exact Or.inr rfl
  · exact Or.inl rfl

/-- A survivor block holding a body block's rewritten text holds its rewritten
occurrences. -/
theorem occAt_body (H : MergeBodyHyps P Q SP R J m σ τ S D G) {x : Origin} {i : Nat}
    (htext : (survOut P J m S D)[i]? = (rwOut P R m (oldFile S D x))[x.2]?) :
    P.occAt (mergeBy R J m S D G) (D.path, i) =
      (P.occAt G ((oldFile S D x).path, x.2)).map (rwOcc R.covers m) := by
  have hc := oldFile_cases S D x
  have hG : oldFile S D x ∈ G := by
    rcases hc with h | h <;> rw [h]
    · exact H.src
    · exact H.dst
  have hmem : oldFile S D x ∈ [S, D] := by rcases hc with h | h <;> simp [h]
  have hfmt : (R.apply m (oldFile S D x)).format = (J.apply m S D).format := by
    rw [H.lawsSD.format _ hmem, H.merger.format]
    rcases hc with h | h <;> rw [h]
    exact H.sameFormat
  rw [occAt_survivor H.toMergeBase, occAt_some H.wf hG]
  have h := congrArg (fun l => l[x.2]?) (H.lawsSD.occ _ hmem)
  simp only [List.getElem?_map] at h
  unfold blockOcc
  change (((survOut P J m S D)[i]?).map _).getD [] = _
  rw [htext]
  unfold rwOut
  rw [← hfmt, ← h]
  cases (P.outline (oldFile S D x).format (oldFile S D x).text)[x.2]? <;> rfl

theorem pageOfBlock_body (H : MergeHyps7b P Q SP R J m σ S D G) (x : Origin) (i : Nat) :
    P.pageOfBlock (mergeBy R J m S D G) (D.path, i) =
      (P.pageOfBlock G ((oldFile S D x).path, x.2)).map (σp σ) := by
  have h1 := fileAt_eq_of_mem H.wf' (survivor_mem H.toMergeOpHyps.toMergeBase)
  rw [H.merger.path] at h1
  obtain ⟨c, j⟩ := x
  cases c
  · simp [pageOfBlock, h1, fileAt_eq_of_mem H.wf H.dst, oldFile, claim_survivor H.toMergeOpHyps.toMergeBase,
      σp_dst H.toMergeOpHyps]
  · simp [pageOfBlock, h1, fileAt_eq_of_mem H.wf H.src, oldFile, claim_survivor H.toMergeOpHyps.toMergeBase,
      σp_src H.toMergeOpHyps]

/-- **MQ3 (body blocks).** A body block of the source or the target keeps its
query inputs at its survivor block, given the forward namespace condition on
it and that its old ancestors' keys still reach the survivor block. The
ancestor condition is an explicit hypothesis: depths in the survivor are not
modelled (`QParams.levels` is opaque), so the model does not derive it. -/
theorem blockForward_body (H : MergeHyps7b P Q SP R J m σ S D G) {x : Origin} {i : Nat}
    (htext : (survOut P J m S D)[i]? = (rwOut P R m (oldFile S D x))[x.2]?)
    (hns : NsForward P R m σ (P.occAt G ((oldFile S D x).path, x.2)))
    (hanc : ∀ a ∈ ancestors Q G ((oldFile S D x).path, x.2), ∀ k ∈ (P.refNames G a).map P.key,
      σ k ∈ P.pathRefKeys Q (mergeBy R J m S D G) (D.path, i)) :
    BlockForward P Q σ G (mergeBy R J m S D G) ((oldFile S D x).path, x.2) (D.path, i) :=
  blockForward_of (occAt_body H.toBody htext) hns hanc (pageOfBlock_body H x i)

/-- **MQ4 (what the stored-query law gives for a body block).** Every body
block of the source or the target has its own survivor block (`bodies_kept`),
and a negation-free, `given`-free query stored there, with σ-consistent
arguments, is stored renamed at that survivor block and still matches the
image of every block it matched whose query inputs go forward. -/
theorem body_stored_forward (H : MergeHyps7b P Q SP R J m σ S D G) :
    ∃ ι : Origin → Nat,
      (∀ x y, IsBody P Q S D x → IsBody P Q S D y → OLt x y → ι x < ι y) ∧
      ∀ x, IsBody P Q S D x →
        ∀ q ∈ storedAt P SP G ((oldFile S D x).path, x.2),
          renameQuery m q ∈ storedAt P SP (mergeBy R J m S D G) (D.path, ι x) ∧
          (Positive q → Defined q → (∀ s ∈ queryArgs q, P.key (m s) = σ (P.key s)) →
            ∀ c c', BlockForward P Q σ G (mergeBy R J m S D G) c c' →
              P.evalNode3 Q G q (.block c) = true →
              P.evalNode3 Q (mergeBy R J m S D G) (renameQuery m q) (.block c') = true) := by
  obtain ⟨ι, hmono, hk⟩ := bodies_kept H.toBody
  refine ⟨ι, hmono, fun x hx q hq => ⟨?_, fun hp hd ha c c' hB hc =>
    block_forward (edges_forward_merge7 H.toMergeOpHyps.toMergeBase) hB q hp hd ha hc⟩⟩
  obtain ⟨_, ho, _, htext⟩ := hk x hx
  have hx2 : x.2 < (rwOut P R m (oldFile S D x)).length := by
    rw [rwOut_length H.toBody (oldFile_cases S D x)]
    obtain ⟨c, j⟩ := x
    cases c <;> simp only [IsBody, oldFile] at hx ⊢ <;> omega
  have hi : ι x < (survOut P J m S D).length := by
    rcases Nat.lt_or_ge (ι x) (survOut P J m S D).length with h | h
    · exact h
    · rw [List.getElem?_eq_none h, List.getElem?_eq_getElem hx2] at htext
      cases htext
  rw [storedAt_body H.toBody hi ho]
  exact List.mem_map_of_mem hq

end Merge7b
end CM
