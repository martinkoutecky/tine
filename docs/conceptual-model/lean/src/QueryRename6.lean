/- Stored-query rewriting, D-20261031-CMQ. No new occurrence/reference rule. -/
import OwnerWitness5 -- Serial extension after the entire inherited tree.

set_option linter.unusedSectionVars false
namespace CM
namespace QueryRename6
open Params

/-- Independent parser boundary; QParams and all existing witness inputs stay unchanged. -/
structure StoredParser (L : Type) where
  extract : Format → String → List (Query L)

deriving instance DecidableEq for Leaf
deriving instance DecidableEq for Query

def renameLeaf {L : Type} (m : String → String) : Leaf L → Leaf L
  | .refs s => .refs (m s)
  | .name s => .name (m s)
  | .page l => .page (renameLeaf m l)
  | .given l => .given l

def renameQuery {L : Type} (m : String → String) : Query L → Query L
  | .leaf l => .leaf (renameLeaf m l)
  | .and q r => .and (renameQuery m q) (renameQuery m r)
  | .or q r => .or (renameQuery m q) (renameQuery m r)
  | .not q => .not (renameQuery m q)

def leafArgs {L : Type} : Leaf L → List String
  | .refs s | .name s => [s]
  | .page l => leafArgs l
  | .given _ => []

def queryArgs {L : Type} (q : Query L) : List String := q.leaves.flatMap leafArgs

def leafDefined {L : Type} : Leaf L → Prop
  | .given _ => False
  | .page l => leafDefined l
  | _ => True

def Defined {L : Type} (q : Query L) : Prop := ∀ l ∈ q.leaves, leafDefined l

variable {K L : Type} [DecidableEq K]

/-- Queries are parsed from the actual block text, independently of `occ`. -/
def storedAt (P : Params K) (S : StoredParser L) (G : Graph) (b : BlockId) : List (Query L) :=
  match fileAt G b.1 with
  | none => []
  | some f => (((P.outline f.format f.text)[b.2]?).map (S.extract f.format)).getD []

/-- Extension of the decided text-rewriter contract, retaining its old projection.
Exact syntax transport is required at every outline position of every file. -/
structure FaithfulOn (P : Params K) (S : StoredParser L) (R : Decided.Rewriter)
    (m : String → String) (G : Graph) : Prop extends Decided.FaithfulOn P R m G where
  stored : ∀ f ∈ G,
    (P.outline (R.apply m f).format (R.apply m f).text).map
      (S.extract (R.apply m f).format) =
    ((P.outline f.format f.text).map (S.extract f.format)).map
      (List.map (renameQuery m))

theorem storedAt_some (P : Params K) (S : StoredParser L) {G : Graph}
    (hw : WF G) {f : File} (hf : f ∈ G) (i : Nat) :
    storedAt P S G (f.path, i) =
      (((P.outline f.format f.text)[i]?).map (S.extract f.format)).getD [] := by
  simp [storedAt, fileAt_eq_of_mem hw hf]

/-- Exact syntax, without freshness, key, ancestry or given-predicate assumptions. -/
theorem storedAt_rewrite (P : Params K) (S : StoredParser L) {R : Decided.Rewriter}
    {m : String → String} {G : Graph} (F : FaithfulOn P S R m G)
    (hw : WF G) (hw' : WF (renameBy R.base m G)) {f : File} (hf : f ∈ G) (i : Nat) :
    storedAt P S (renameBy R.base m G) ((R.apply m f).path, i) =
      (storedAt P S G (f.path, i)).map (renameQuery m) := by
  rw [storedAt_some P S hw' (f := R.apply m f) (List.mem_map_of_mem hf), storedAt_some P S hw hf]
  have h := congrArg (fun xs => xs[i]?) (F.stored f hf)
  simp only [List.getElem?_map] at h
  change (((P.outline (R.apply m f).format (R.apply m f).text)[i]?).map
    (S.extract (R.apply m f).format)) = _ at h
  rw [h]
  cases (P.outline f.format f.text)[i]? <;> rfl

theorem storedAt_blockMap (P : Params K) (S : StoredParser L) {R : Decided.Rewriter}
    {m : String → String} {G : Graph} (F : FaithfulOn P S R m G)
    (hw : WF G) (hw' : WF (renameBy R.base m G)) {b : BlockId} (hb : b ∈ P.blocks G) :
    storedAt P S (renameBy R.base m G) (blockMap R.base m G b) =
      (storedAt P S G b).map (renameQuery m) := by
  obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
  have he : b = (f.path, b.2) := Prod.ext hp.symm rfl
  rw [he, blockMap_eq hw hf]
  exact storedAt_rewrite P S F hw hw' hf b.2

def nodeMap (R : CM.Rewriter) (m : String → String) (σ : K → K) (G : Graph) : Node K → Node K
  | .page p => .page (σp σ p)
  | .block b => .block (blockMap R m G b)

/-- Query arguments need separation only from graph keys, not global injectivity. -/
def ArgSafe (P : Params K) (m : String → String) (σ : K → K) (G : Graph) (s : String) : Prop :=
  P.key (m s) = σ (P.key s) ∧
  ∀ k, P.InPlay G k → σ (P.key s) = σ k → P.key s = k

theorem argSafe_inPlay {P : Params K} {R : CM.Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G) {s : String}
    (hs : P.InPlay G (P.key s)) (hm : P.key (m s) = σ (P.key s)) :
    ArgSafe P m σ G s := ⟨hm, fun _ hk he => inPlay_inj H.keys hs hk he⟩

/-- An external queried key need not be a page, alias endpoint, or occurrence. -/
theorem aliasClass_arg {P : Params K} {R : CM.Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G) {s : String}
    (hs : ArgSafe P m σ G s) {k : K} (hk : P.InPlay G k) :
    P.aliasClass (renameBy R m G) (P.key (m s)) (σ k) ↔ P.aliasClass G (P.key s) k := by
  rw [hs.1]
  unfold aliasClass cls
  rw [Reach.congr (aliasEdges_rename H)]
  constructor
  · intro h
    have hS : ∀ e ∈ P.aliasEdges G, P.InPlay G e.1 ∧ P.InPlay G e.2 :=
      fun e he => ⟨Or.inr (Or.inr ⟨e, he, Or.inl rfl⟩), Or.inr (Or.inr ⟨e, he, Or.inr rfl⟩)⟩
    obtain ⟨c, hc, hr, he⟩ := Reach.map_bwd σ (P.InPlay G) hS
      (fun _ _ ha hb => inPlay_inj H.keys ha hb) hk (Reach.symm h)
    have ec : P.key s = c := hs.2 c hc he.symm
    rw [← ec] at hr
    exact Reach.symm hr
  · exact Reach.map_fwd σ

theorem ancestors_blocks (P : Params K) (Q : QParams K L) {G : Graph}
    (_hw : WF G) {b : BlockId} (hb : b ∈ P.blocks G) {a : BlockId}
    (ha : a ∈ ancestors Q G b) : a ∈ P.blocks G := by
  obtain ⟨f, hf, ho, hp, hi⟩ := P.mem_blocks.mp hb
  obtain ⟨j, hj, he⟩ := List.mem_map.mp ha
  obtain ⟨hj, _⟩ := List.mem_filter.mp hj
  have hj := List.mem_range.mp hj
  subst a
  exact P.mem_blocks.mpr ⟨f, hf, ho, hp, Nat.lt_trans hj hi⟩

theorem pageOfBlock_rewrite {P : Params K} {R : CM.Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G) {b : BlockId}
    (hb : b ∈ P.blocks G) :
    P.pageOfBlock (renameBy R m G) (blockMap R m G b) = (P.pageOfBlock G b).map (σp σ) := by
  obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
  have he : b = (f.path, b.2) := Prod.ext hp.symm rfl
  rw [he, blockMap_eq H.wf hf]
  simp [pageOfBlock, fileAt_eq_of_mem H.wf hf,
    fileAt_eq_of_mem H.wf' (List.mem_map_of_mem hf), claim_apply H.laws hf]

theorem ancestors_rewrite {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    {b : BlockId} (hb : b ∈ P.blocks G) :
    ancestors Q (renameBy R m G) (blockMap R m G b) =
      (ancestors Q G b).map (blockMap R m G) := by
  obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
  have he : b = (f.path, b.2) := Prod.ext hp.symm rfl
  rw [he, blockMap_eq H.wf hf]
  simp only [ancestors, fileAt_eq_of_mem H.wf hf,
    fileAt_eq_of_mem H.wf' (List.mem_map_of_mem hf), hd f hf, List.map_map]
  apply List.map_congr_left
  intro i _
  exact (blockMap_eq H.wf hf i).symm

theorem refKeys_at_rewrite {P : Params K} {R : CM.Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) {b : BlockId} (hb : b ∈ P.blocks G) (k : K) :
    k ∈ (P.refNames (renameBy R m G) (blockMap R m G b)).map P.key ↔
      ∃ u ∈ (P.refNames G b).map P.key, σ u = k := by
  obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
  have he : b = (f.path, b.2) := Prod.ext hp.symm rfl
  rw [he, blockMap_eq H.wf hf]
  rw [List.mem_map, refName_keys_rename H hf b.2 (he ▸ hx b hb) k]
  constructor
  · rintro ⟨u, hu, he⟩; exact ⟨P.key u, List.mem_map_of_mem hu, he⟩
  · rintro ⟨u, hu, he⟩
    obtain ⟨s, hs, rfl⟩ := List.mem_map.mp hu
    exact ⟨s, hs, he⟩

theorem pathKeys_inPlay (P : Params K) (Q : QParams K L) {G : Graph}
    (hw : WF G) {b : BlockId} (hb : b ∈ P.blocks G) {k : K}
    (hk : k ∈ P.pathRefKeys Q G b) : P.InPlay G k := by
  simp only [pathRefKeys, List.mem_append, List.mem_flatMap, List.mem_map] at hk
  rcases hk with ((⟨s, hs, rfl⟩ | ⟨a, ha, s, hs, rfl⟩) | hk)
  · exact inPlay_refName hb hs
  · exact inPlay_refName (ancestors_blocks P Q hw hb ha) hs
  · obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
    simp only [pageOfBlock, ← hp, fileAt_eq_of_mem hw hf, Option.map_some,
      Option.toList_some, List.mem_singleton] at hk
    rw [hk]; exact P.inPlay_claim hf

theorem pathKeys_rewrite {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    {b : BlockId} (hb : b ∈ P.blocks G) (k : K) :
    k ∈ P.pathRefKeys Q (renameBy R m G) (blockMap R m G b) ↔
      ∃ u ∈ P.pathRefKeys Q G b, σ u = k := by
  have hanc := ancestors_rewrite Q H hd hb
  have hpob := pageOfBlock_rewrite H hb
  simp only [pathRefKeys, hanc, hpob, List.mem_append, List.mem_flatMap, List.mem_map,
    refKeys_at_rewrite H hx hb] 
  constructor
  · rintro ((h | ⟨a', ⟨a, ha, rfl⟩, hk⟩) | hk)
    · obtain ⟨u, hu, he⟩ := h; exact ⟨u, Or.inl (Or.inl hu), he⟩
    · obtain ⟨u, hu, he⟩ := (refKeys_at_rewrite H hx (ancestors_blocks P Q H.wf hb ha) k).mp
        (List.mem_map.mpr hk)
      exact ⟨u, Or.inl (Or.inr ⟨a, ha, List.mem_map.mp hu⟩), he⟩
    · cases hp : P.pageOfBlock G b with
      | none => simp [hp] at hk
      | some p =>
        simp only [hp, Option.map_some, Option.toList_some, List.mem_singleton] at hk
        exact ⟨p.2, Or.inr (by simp), hk.symm⟩
  · rintro ⟨u, ((hu | ⟨a, ha, hu⟩) | hu), he⟩
    · exact Or.inl (Or.inl ⟨u, hu, he⟩)
    · exact Or.inl (Or.inr ⟨blockMap R m G a, ⟨a, ha, rfl⟩,
        List.mem_map.mp ((refKeys_at_rewrite H hx (ancestors_blocks P Q H.wf hb ha) k).mpr
          ⟨u, List.mem_map.mpr hu, he⟩)⟩)
    · apply Or.inr
      cases hp : P.pageOfBlock G b with
      | none => simp [hp] at hu
      | some p =>
        simp only [hp, Option.map_some, Option.toList_some, List.mem_singleton] at hu ⊢
        simpa [hu, σp] using he.symm

/-- Only given predicates actually present in a query need transport laws.
Their opaque tokens are unchanged; their readings are not automatically invariant. -/
def GivenSafe (P : Params K) (Q : QParams K L) (R : CM.Rewriter)
    (m : String → String) (σ : K → K) (G : Graph) : Leaf L → Prop
  | .given l =>
    (∀ p, P.InPlay G p.2 → Q.pageLeaf l (renameBy R m G) (σp σ p) = Q.pageLeaf l G p) ∧
    (∀ b ∈ P.blocks G, Q.blockLeaf l (renameBy R m G) (blockMap R m G b) = Q.blockLeaf l G b)
  | .page l => GivenSafe P Q R m σ G l
  | _ => True

theorem defined_givenSafe (P : Params K) (Q : QParams K L) (R : CM.Rewriter)
    (m : String → String) (σ : K → K) (G : Graph) (l : Leaf L) (hl : leafDefined l) :
    GivenSafe P Q R m σ G l := by
  induction l with
  | refs | name => trivial
  | page l ih => exact ih hl
  | given => exact False.elim hl

theorem leafPage_transport {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (l : Leaf L)
    (ha : ∀ s ∈ leafArgs l, ArgSafe P m σ G s)
    (hg : GivenSafe P Q R m σ G l) {p : Page K} (hp : P.InPlay G p.2) :
    P.leafPage Q (renameBy R m G) (renameLeaf m l) (σp σ p) = P.leafPage Q G l p := by
  induction l with
  | refs => rfl
  | name s =>
    have hs := ha s (by simp [leafArgs])
    simp only [renameLeaf, leafPage, σp, hs.1]
    apply decide_eq_decide.mpr
    exact ⟨hs.2 p.2 hp, congrArg σ⟩
  | page l ih => exact ih ha hg
  | given l => exact hg.1 p hp

theorem leafBlock_transport {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    (l : Leaf L) (ha : ∀ s ∈ leafArgs l, ArgSafe P m σ G s)
    (hg : GivenSafe P Q R m σ G l) {b : BlockId} (hb : b ∈ P.blocks G) :
    P.leafBlock3 Q (renameBy R m G) (renameLeaf m l) (blockMap R m G b) =
      P.leafBlock3 Q G l b := by
  cases l with
  | refs s =>
    have hs := ha s (by simp [leafArgs])
    apply Bool.eq_iff_iff.mpr
    simp only [renameLeaf, leafBlock3, List.any_eq_true, sameCls_iff]
    constructor
    · rintro ⟨k', hk', hc⟩
      obtain ⟨k, hk, rfl⟩ := (pathKeys_rewrite Q H hx hd hb k').mp hk'
      exact ⟨k, hk, (aliasClass_arg H hs (pathKeys_inPlay P Q H.wf hb hk)).mp hc⟩
    · rintro ⟨k, hk, hc⟩
      exact ⟨σ k, (pathKeys_rewrite Q H hx hd hb (σ k)).mpr ⟨k, hk, rfl⟩,
        (aliasClass_arg H hs (pathKeys_inPlay P Q H.wf hb hk)).mpr hc⟩
  | name => rfl
  | page l =>
    obtain ⟨f, hf, _, hp, _⟩ := P.mem_blocks.mp hb
    have hpob : P.pageOfBlock G b = some (P.claim f) := by
      simp [pageOfBlock, ← hp, fileAt_eq_of_mem H.wf hf]
    simp only [renameLeaf, leafBlock3, pageOfBlock_rewrite H hb, hpob, Option.map_some]
    exact leafPage_transport Q l ha hg (P.inPlay_claim hf)
  | given l => exact hg.2 b hb

theorem eval_transport {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    (q : Query L) (ha : ∀ s ∈ queryArgs q, ArgSafe P m σ G s)
    (hg : ∀ l ∈ q.leaves, GivenSafe P Q R m σ G l)
    {a : Anchor} {n : Node K} (hn : n ∈ P.univ true G a) :
    P.evalNode3 Q (renameBy R m G) (renameQuery m q) (nodeMap R m σ G n) =
      P.evalNode3 Q G q n := by
  have hl : ∀ l ∈ q.leaves,
      P.leafAt3 Q (renameBy R m G) (renameLeaf m l) (nodeMap R m σ G n) =
        P.leafAt3 Q G l n := by
    intro l hl
    have hargs : ∀ s ∈ leafArgs l, ArgSafe P m σ G s :=
      fun s hs => ha s (List.mem_flatMap.mpr ⟨l, hl, hs⟩)
    cases a with
    | block =>
      obtain ⟨b, hb, rfl⟩ := List.mem_map.mp hn
      exact leafBlock_transport Q H hx hd l hargs (hg l hl) hb
    | page =>
      obtain ⟨p, hp, rfl⟩ := List.mem_map.mp hn
      exact leafPage_transport Q l hargs (hg l hl) (P.inPlay_of_pagesR hp)
  unfold evalNode3
  clear hn ha hg
  induction q with
  | leaf l => exact hl l (by simp [Query.leaves])
  | and q r ihq ihr | or q r ihq ihr =>
    simp only [renameQuery, Query.bval]
    rw [ihq (fun l hm => hl l (by simp [Query.leaves, hm])),
      ihr (fun l hm => hl l (by simp [Query.leaves, hm]))]
  | not q ih =>
    simp only [renameQuery, Query.bval]
    rw [ih hl]

theorem univ_image {P : Params K} {R : CM.Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (a : Anchor) (n' : Node K) :
    n' ∈ P.univ true (renameBy R m G) a ↔
      ∃ n ∈ P.univ true G a, nodeMap R m σ G n = n' := by
  cases a with
  | block =>
    simp only [univ, List.mem_map]
    constructor
    · rintro ⟨b', hb', rfl⟩
      obtain ⟨b, hb, rfl⟩ := (blocks_rename H b').mp hb'
      exact ⟨.block b, ⟨b, hb, rfl⟩, rfl⟩
    · rintro ⟨n, ⟨b, hb, rfl⟩, rfl⟩
      exact ⟨blockMap R m G b, (blocks_rename H _).mpr ⟨b, hb, rfl⟩, rfl⟩
  | page =>
    simp only [univ, List.mem_map]
    constructor
    · rintro ⟨p', hp', rfl⟩
      obtain ⟨p, hp, rfl⟩ := pagesR_rename_onto H hx hp'
      exact ⟨.page p, ⟨p, hp, rfl⟩, rfl⟩
    · rintro ⟨n, ⟨p, hp, rfl⟩, rfl⟩
      exact ⟨σp σ p, pagesR_rename_fwd H hx hp, rfl⟩

/-- Full denotation image, for either anchor, with Boolean negation included. -/
theorem den_image {P : Params K} (Q : QParams K L) {R : CM.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    (q : Query L) (ha : ∀ s ∈ queryArgs q, ArgSafe P m σ G s)
    (hg : ∀ l ∈ q.leaves, GivenSafe P Q R m σ G l) (a : Anchor) (n' : Node K) :
    n' ∈ Decided.den P Q (renameBy R m G) a (renameQuery m q) ↔
      ∃ n ∈ Decided.den P Q G a q, nodeMap R m σ G n = n' := by
  simp only [Decided.den, den3, P.mem_denU3 Q]
  rw [univ_image H hx]
  constructor
  · rintro ⟨⟨n, hn, rfl⟩, he⟩
    rw [eval_transport Q H hx hd q ha hg hn] at he
    exact ⟨n, ⟨hn, he⟩, rfl⟩
  · rintro ⟨n, ⟨hn, he⟩, rfl⟩
    exact ⟨⟨n, hn, rfl⟩, (eval_transport Q H hx hd q ha hg hn).trans he⟩

/-- Read a particular stored query from the image block, then evaluate THAT query. -/
theorem stored_den_image {P : Params K} (Q : QParams K L) (S : StoredParser L) {R : Decided.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R.base m σ G)
    (F : FaithfulOn P S R m G) (hx : P.GraphExceptionFree R.base σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    {b : BlockId} (hb : b ∈ P.blocks G) {i : Nat} {q : Query L}
    (hq : (storedAt P S G b)[i]? = some q)
    (ha : ∀ s ∈ queryArgs q, ArgSafe P m σ G s)
    (hg : ∀ l ∈ q.leaves, GivenSafe P Q R.base m σ G l) :
    ∃ q', (storedAt P S (renameBy R.base m G) (blockMap R.base m G b))[i]? = some q' ∧
      q' = renameQuery m q ∧ ∀ a n',
      n' ∈ Decided.den P Q (renameBy R.base m G) a q' ↔
        ∃ n ∈ Decided.den P Q G a q, nodeMap R.base m σ G n = n' := by
  refine ⟨renameQuery m q, ?_, rfl, den_image Q H hx hd q ha hg⟩
  rw [storedAt_blockMap P S F H.wf H.wf' hb, List.getElem?_map, hq]
  rfl

theorem stored_defined_image {P : Params K} (Q : QParams K L) (S : StoredParser L) {R : Decided.Rewriter}
    {m : String → String} {σ : K → K} {G : Graph} (H : P.RenameHyps R.base m σ G)
    (F : FaithfulOn P S R m G) (hx : P.GraphExceptionFree R.base σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text)
    {b : BlockId} (hb : b ∈ P.blocks G) {i : Nat} {q : Query L}
    (hq : (storedAt P S G b)[i]? = some q)
    (ha : ∀ s ∈ queryArgs q, ArgSafe P m σ G s) (hdef : Defined q) :
    ∃ q', (storedAt P S (renameBy R.base m G) (blockMap R.base m G b))[i]? = some q' ∧
      q' = renameQuery m q ∧ ∀ a n',
      n' ∈ Decided.den P Q (renameBy R.base m G) a q' ↔
        ∃ n ∈ Decided.den P Q G a q, nodeMap R.base m σ G n = n' :=
  stored_den_image Q S H F hx hd hb hq ha
    (fun l hl => defined_givenSafe P Q R.base m σ G l (hdef l hl))

/-- Opaque search tokens are never changed by the spelling substitution. -/
theorem given_fixed (m : String → String) (l : L) :
    renameLeaf m (.given l) = .given l := rfl

/-- Exactly the page-name positions, including below page hops. -/
theorem leaf_args_renamed (m : String → String) (l : Leaf L) :
    leafArgs (renameLeaf m l) = (leafArgs l).map m := by
  induction l with
  | refs | name | given => rfl
  | page l ih => exact ih

/-- Actual successful decided rename, rather than only an arbitrary rewrite.
Success identifies G'; it does not discharge any preservation hypothesis. -/
theorem stored_after_success {P : Params K} (Q : QParams K L) (S : StoredParser L)
    {R : Decided.Rewriter} {G G' : Graph} {old new : String} {σ : K → K}
    (hs : Decided.rename P R G old new = some G')
    (H : P.RenameHyps R.base (nsSubst P.key P.urlHasOrigin old new) σ G)
    (F : FaithfulOn P S R (nsSubst P.key P.urlHasOrigin old new) G)
    (hx : P.GraphExceptionFree R.base σ G)
    (hd : ∀ f ∈ G, Q.levels (R.apply (nsSubst P.key P.urlHasOrigin old new) f).format
      (R.apply (nsSubst P.key P.urlHasOrigin old new) f).text = Q.levels f.format f.text)
    {b : BlockId} (hb : b ∈ P.blocks G) {i : Nat} {q : Query L}
    (hq : (storedAt P S G b)[i]? = some q)
    (ha : ∀ s ∈ queryArgs q, ArgSafe P (nsSubst P.key P.urlHasOrigin old new) σ G s)
    (hg : ∀ l ∈ q.leaves, GivenSafe P Q R.base (nsSubst P.key P.urlHasOrigin old new) σ G l) :
    ∃ q', (storedAt P S G' (blockMap R.base (nsSubst P.key P.urlHasOrigin old new) G b))[i]? = some q' ∧
      q' = renameQuery (nsSubst P.key P.urlHasOrigin old new) q ∧ ∀ a n',
      n' ∈ Decided.den P Q G' a q' ↔
        ∃ n ∈ Decided.den P Q G a q,
          nodeMap R.base (nsSubst P.key P.urlHasOrigin old new) σ G n = n' := by
  have he : G' = renameBy R.base (nsSubst P.key P.urlHasOrigin old new) G :=
    Decided.rename_success_graph P hs
  rw [he]
  exact stored_den_image Q S H F hx hd hb hq ha hg

end QueryRename6
end CM
