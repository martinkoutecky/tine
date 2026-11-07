/- Decided round-4 headlines. Earlier statements and refutations are retained
as LegacyHeadlines; obsolete definitions carry Legacy/LegacyA names. The full
checked dependency cones of ALL headlines and ALL round-3/4 witnesses are audited. -/
import Lean
import LegacyHeadlines
import Witnesses4

namespace CM
namespace Headlines
open CM CM.Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## T1: resolution is well defined -/

/-- T1a. `resolve` and `concept` depend on the graph only as a set of files. -/
theorem T1_set (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (s : String) :
    P.resolve G₁ s = P.resolve G₂ s ∧ P.concept G₁ s = P.concept G₂ s :=
  ⟨resolve_sameFiles hP h s, concept_sameFiles hP h s⟩

/-- T1b. In particular, not on the listing order. -/
theorem T1_perm (hP : P.Lawful) {G₁ G₂ : Graph} (h : G₁.Perm G₂) (s : String) :
    P.resolve G₁ s = P.resolve G₂ s ∧ P.concept G₁ s = P.concept G₂ s :=
  resolve_perm hP h s

/-- T1c. They depend on a spelling only through `key`. -/
theorem T1_key (G : Graph) {s t : String} (h : P.key s = P.key t) :
    P.resolve G s = P.resolve G t ∧ P.concept G s = P.concept G t :=
  resolve_key G h

/-- T1d. The concept of a spelling is the alias class of its own key (whatever `resolve` picks). -/
theorem T1_concept_class (G : Graph) (s : String) : P.concept G s = P.aliasClass G (P.key s) :=
  P.concept_eq_class G s

/-- T1e. The Linked References relation is a function of the set of files (paths unique). -/
theorem T1_references (hP : P.Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂)
    (b : BlockId) (t : Page K) : P.References G₁ b t ↔ P.References G₂ b t :=
  references_sameFiles hP hw h b t

/-- T1f. The executable forms decide the propositional ones. -/
theorem T1_executable (G : Graph) (s : String) (k : K) (b : BlockId) (t : Page K) :
    (P.conceptB G s k = true ↔ P.concept G s k) ∧ (P.referencesB G b t = true ↔ P.References G b t) :=
  ⟨P.conceptB_iff G s k, P.referencesB_iff G b t⟩

/-- T1g. The Q9 page and node universes depend only on the file set. -/
theorem T1_pages (hP : P.Lawful) {G H : Graph} (hw : WF H) (h : SameFiles G H) :
    (∀ p, p ∈ Decided.pages P G ↔ p ∈ Decided.pages P H) ∧
    (∀ n, n ∈ Decided.nodes P G ↔ n ∈ Decided.nodes P H) := by
  refine ⟨Decided.pages_sameFiles P hP hw h, ?_⟩
  intro n
  cases n with
  | page p => simpa [Decided.nodes] using Decided.pages_sameFiles P hP hw h p
  | block b => simpa [Decided.nodes] using blocks_sameFiles hP h b

/-! ## T2: classes (for any edge list; `aliasEdges G` is one) -/

variable {α : Type}

/-- T2a. Same-class is an equivalence relation, so classes partition the keys: every key is in its
own class, and two classes sharing a member are equal. -/
theorem T2_partition (E : List (α × α)) :
    (∀ k, cls E k k) ∧ (∀ a b, cls E a b → cls E b a) ∧ (∀ a b c, cls E a b → cls E b c → cls E a c) ∧
    (∀ k k' z, cls E k z → cls E k' z → cls E k = cls E k') :=
  ⟨cls_self E, fun _ _ => Reach.symm, fun _ _ _ => Reach.trans, fun _ _ _ => cls_eq_of_common E⟩

/-- T2b. A key no edge mentions is its own singleton class. -/
theorem T2_isolated {E : List (α × α)} {k : α} (h : ∀ e ∈ E, e.1 ≠ k ∧ e.2 ≠ k) (y : α) :
    cls E k y ↔ y = k := cls_isolated h y

/-- T2c. Adding the edge `(a, b)`: no class splits; a key outside the classes of `a` and `b` keeps
its class; a key in either gets their union (so at most two classes merge). -/
theorem T2_insert (E : List (α × α)) (a b : α) :
    (∀ x y, cls E x y → cls ((a, b) :: E) x y) ∧
    (∀ x, ¬ cls E x a → ¬ cls E x b → cls ((a, b) :: E) x = cls E x) ∧
    (∀ x, cls E x a ∨ cls E x b → cls ((a, b) :: E) x = fun y => cls E a y ∨ cls E b y) :=
  ⟨fun _ _ => cls_insert_mono E (a, b), fun _ ha hb => cls_insert_other E ha hb,
   fun _ hx => cls_insert_merged E hx⟩

/-- T2d. Removing the edge `(a, b)` (any `E` whose members are `E₀`'s plus `(a, b)`): no classes
merge; a key outside the class of `a` keeps its class; that class splits into at most the new classes
of `a` and `b`. -/
theorem T2_remove {E E₀ : List (α × α)} {a b : α} (h : RemovesEdge E E₀ a b) :
    (∀ x y, cls E₀ x y → cls E x y) ∧
    (∀ x, ¬ cls E x a → cls E₀ x = cls E x) ∧
    (∀ y, cls E a y → cls E₀ a y ∨ cls E₀ b y) :=
  ⟨fun _ _ => cls_remove_sub h, fun _ hx => cls_remove_other h hx, fun _ hy => cls_remove_split h hy⟩

/-- T2e. Classes depend on the edge list only through its members, and `sameCls` decides them. -/
theorem T2_decide [DecidableEq α] (E E' : List (α × α)) (hE : ∀ e, e ∈ E ↔ e ∈ E') (x y : α) :
    (cls E x y ↔ cls E' x y) ∧ (sameCls E x y = true ↔ cls E x y) :=
  ⟨Reach.congr hE, sameCls_iff E x y⟩

/-! ## T4: locality -/

/-- T4a. Occurrences change only on blocks of the file at `π`. -/
theorem T4_occ {π : String} {G G' : Graph} (hw : WF G') (hD : DiffAt π G G') (b : BlockId)
    (hb : b.1 ≠ π) : P.occAt G b = P.occAt G' b :=
  occAt_local hw hD b hb

/-- T4b. A block of another file stays or stays out of the block set unless that file claims a page
a touched file claims. -/
theorem T4_blocks (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {b : BlockId}
    (hb : b.1 ≠ π)
    (hp : ∀ g, (g ∈ G ∨ g ∈ G') → g.path = b.1 →
      ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → P.claim h ≠ P.claim g) :
    b ∈ P.blocks G ↔ b ∈ P.blocks G' :=
  blocks_local hP hD hb hp

/-- T4c (sharp form). `resolve s` and `concept s` are unchanged when the alias class of `key s`,
before and after, contains no key claimed by the file at `π`. -/
theorem T4_resolve (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (s : String)
    (h1 : ∀ y, P.aliasClass G (P.key s) y → ¬ P.Affected π G G' y)
    (h2 : ∀ y, P.aliasClass G' (P.key s) y → ¬ P.Affected π G G' y) :
    P.resolve G s = P.resolve G' s ∧ P.concept G s = P.concept G' s :=
  resolve_local hP hD s h1 h2

/-- T4c (SPEC §7 wording): the same with "claims or declares". -/
theorem T4_resolve_spec (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (s : String)
    (h1 : ∀ y, P.aliasClass G (P.key s) y → ¬ P.ClaimedOrDeclared π G G' y)
    (h2 : ∀ y, P.aliasClass G' (P.key s) y → ¬ P.ClaimedOrDeclared π G G' y) :
    P.resolve G s = P.resolve G' s ∧ P.concept G s = P.concept G' s :=
  resolve_local_spec hP hD s h1 h2

/-- T4d (incremental = rebuild). The alias adjacency updated by the delta (drop the edges declared
from the affected keys; add those the new owners of the affected pages declare) has exactly the edges
of the rebuilt one, so every class computed from it is the rebuilt class. -/
theorem T4_delta (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') :
    (∀ e, e ∈ deltaEdges P (P.aliasEdges G) (affectedKeys P π G G') G' ↔ e ∈ P.aliasEdges G') ∧
    (∀ k, cls (deltaEdges P (P.aliasEdges G) (affectedKeys P π G G') G') k = P.aliasClass G' k) :=
  ⟨delta_eq_rebuild hP hD, delta_class_eq_rebuild hP hD⟩



/- Fresh, namespace-compatible transport, with decided occurrence coverage. -/
section T3
variable {P} {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}

/-- T3a. Through covered (rewritten) occurrence kinds, references are carried by `σ`, with no
exception-freeness hypothesis. -/
theorem T3_covered (H : P.RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ P.blocks G) {t : Page K}
    (ht : P.InPlay G t.2) :
    P.RefsBy Decided.covers G b t ↔ P.RefsBy Decided.covers (renameBy R.base m G) (blockMap R.base m G b) (σp σ t) :=
  refsBy_covers_rename H hb ht

/-- T3b. For an exception-free block, all its references are carried by `σ`. -/
theorem T3_references (H : P.RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R.base σ G b) {t : Page K} (ht : P.InPlay G t.2) :
    P.References G b t ↔ P.References (renameBy R.base m G) (blockMap R.base m G b) (σp σ t) :=
  references_rename H hb hx ht

/-- T3b (SPEC's form): `refs(G', b') = σ(refs(G, b))` as a set of concepts. -/
theorem T3_refs (H : P.RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R.base σ G b) :
    P.refs (renameBy R.base m G) (blockMap R.base m G b) = fun C => ∃ C₀, P.refs G b C₀ ∧ C = img σ C₀ :=
  refs_rename H hb hx

/-- T3c. With the whole graph exception-free, `σp σ` is a bijection `pagesR r G → pagesR r G'`,
under either reading `r` of P1. -/
theorem T3_pages (H : P.RenameHyps R.base m σ G) (hx : P.GraphExceptionFree R.base σ G) (r : Bool) :
    (∀ p ∈ P.pagesR r G, σp σ p ∈ P.pagesR r (renameBy R.base m G)) ∧
    (∀ q ∈ P.pagesR r (renameBy R.base m G), ∃ p ∈ P.pagesR r G, σp σ p = q) ∧
    (∀ p ∈ P.pagesR r G, ∀ q ∈ P.pagesR r G, σp σ p = σp σ q → p = q) :=
  ⟨fun _ hp => pagesR_rename_fwd H hx hp, fun _ hq => pagesR_rename_onto H hx hq,
   fun _ hp _ hq e => pagesR_rename_inj H hp hq e⟩

/-- T3d. The blocks of the renamed page are the images of the old page's blocks. -/
theorem T3_blocks (H : P.RenameHyps R.base m σ G) {p : Page K} (hp : P.InPlay G p.2) :
    P.pageBlocks (renameBy R.base m G) (σp σ p) = (P.pageBlocks G p).map (blockMap R.base m G) :=
  pageBlocks_rename H hp

/-- T3d (SPEC's form). A page that does not move keeps exactly its blocks. -/
theorem T3_fixed (H : P.RenameHyps R.base m σ G) {p : Page K} (hp : P.InPlay G p.2) (hfix : σ p.2 = p.2) :
    P.pageBlocks (renameBy R.base m G) p = P.pageBlocks G p :=
  pageBlocks_fixed H hp hfix

end T3

section T5
variable {L : Type} (Q : QParams K L)

/-- T5a. `⟦·⟧` over any universe is a Boolean homomorphism. -/
theorem T5_bool {U : List (Node K)} {G : Graph} (q r : Query L) (n : Node K) :
    (n ∈ P.denU3 Q U G (.and q r) ↔ n ∈ P.denU3 Q U G q ∧ n ∈ P.denU3 Q U G r) ∧
    (n ∈ P.denU3 Q U G (.or q r) ↔ n ∈ P.denU3 Q U G q ∨ n ∈ P.denU3 Q U G r) ∧
    (n ∈ P.denU3 Q U G (.not q) ↔ n ∈ U ∧ n ∉ P.denU3 Q U G q) :=
  ⟨P.denU_and3 Q n, P.denU_or3 Q n, P.denU_not3 Q n⟩

/-- T5a. Queries with the same truth table have the same denotation. -/
theorem T5_truth_table {U : List (Node K)} {G : Graph} {q q' : Query L} (h : ∀ v, q.bval v = q'.bval v) :
    P.denU3 Q U G q = P.denU3 Q U G q' :=
  P.denU_bool3 Q h

/-- T5b. Every view displays exactly `⟦q⟧`. -/
theorem T5_views (v : View (Node K)) (r : Bool) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ displayed (v.apply (P.den3 Q r G a q)) ↔ n ∈ P.den3 Q r G a q :=
  P.view_den3 Q v r G a q n

/-- T5c. The model computes the kind-uniform leaves. -/
theorem T5_computes {r : Bool} {G : Graph} {ls : List (Leaf L)} {l : Leaf L} :
    l ∈ P.kindUniformLeaves3 Q r G ls ↔ l ∈ ls ∧ P.KindUniform3 Q r G l :=
  P.mem_kindUniformLeaves3 Q

/-- T5c. Which model-defined leaves are kind-uniform. -/
theorem T5_leaves {r : Bool} {G : Graph} (hw : WF G) (l : Leaf L) (s : String) :
    P.KindUniform3 Q r G (.page l) ∧
    (P.KindUniform3 Q r G (.refs s) ↔
      ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b → ∀ k ∈ P.pathRefKeys Q G b, ¬ P.aliasClass G (P.key s) k) ∧
    (P.KindUniform3 Q r G (.name s) ↔ ∀ p ∈ P.pagesR r G, P.preBlockOf Q G p ≠ none → P.key s ≠ p.2) :=
  ⟨P.page_kindUniform3 Q hw l, P.refs_kindUniform_iff3 Q, P.name_kindUniform_iff3 Q⟩

/-- T5d. A query whose leaves are all kind-uniform treats a page and its pre-block alike. -/
theorem T5_uniform {r : Bool} {G : Graph} (hw : WF G) {q : Query L}
    (hu : ∀ l ∈ q.leaves, P.KindUniform3 Q r G l) {p : Page K} (hp : p ∈ P.pagesR r G) {b : BlockId}
    (hb : P.preBlockOf Q G p = some b) :
    Node.page p ∈ P.den3 Q r G .page q ↔ Node.block b ∈ P.den3 Q r G .block q :=
  P.query_uniform3 Q hw hu hp hb

end T5

section T6
variable {P} {R : Decided.Rewriter} {m : String → String} {σ : K → K}

theorem T6_link {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S T : Page K} (hS : S ∈ P.pagesR r G) (hT : T ∈ P.pagesR r G) :
    P.Link r fav G S T ↔ P.Link r fav (renameBy R.base m G) (σp σ S) (σp σ T) :=
  link_rename H hx hfav hS hT

theorem T6_link_onto {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S' T' : Page K} (h : P.Link r fav (renameBy R.base m G) S' T') :
    ∃ S ∈ P.pagesR r G, ∃ T ∈ P.pagesR r G, σp σ S = S' ∧ σp σ T = T' ∧ P.Link r fav G S T :=
  link_rename_onto H hx hfav h

theorem T6_near {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ P.pagesR r G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ P.pagesR r G) :
    P.NearP r fav G d k X T ↔ P.NearP r fav (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  near_rename H hx hfav hX d k hT

/-- T6 (executable). The neighbourhoods are computed. -/
theorem T6_computes {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ P.nearL r fav G d k X ↔ P.NearP r fav G d k X T :=
  mem_nearL d k X T

/-- T6. A link to one alias-class member is a link to any other (same class, distinct from the source). -/
theorem T6_class {r : Bool} {fav : Page K} {G : Graph} {S T T' : Page K} (hT : T ∈ P.pagesR r G)
    (hT' : T' ∈ P.pagesR r G) (hc : P.aliasClass G T.2 = P.aliasClass G T'.2) (hs : S = T ↔ S = T') :
    P.Link r fav G S T ↔ P.Link r fav G S T' :=
  link_class hT hT' hc hs

end T6


section T6more
variable {P}

/-- T6. `link` depends on the spelling naming `T` only through `key`. -/
theorem T6_key {r : Bool} {fav : Page K} {G : Graph} {S : Page K} {s t : String} (h : P.key s = P.key t) :
    P.Link r fav G S (P.resolve G s) ↔ P.Link r fav G S (P.resolve G t) :=
  link_key h

/-- T6. `near` is monotone in `k`. -/
theorem T6_mono {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) {k k' : Nat} (hk : k ≤ k')
    (X : List (Page K)) (T : Page K) : P.NearP r fav G d k X T → P.NearP r fav G d k' X T :=
  near_mono d hk X T

/-- T6. `near X k both = out k X ∪ in k X` (by definition; `Witnesses2.GL3_near_ne_ball`: not the ball). -/
theorem T6_both {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    P.NearP r fav G .both k X T ↔ P.OutP r fav G k X T ∨ P.InP r fav G k X T :=
  near_both k X T

end T6more

/-- T3 for an actually accepted decided rename, under the explicit fresh
transport hypotheses. Exceptions are per-block, rather than graph-global. -/
theorem T3_rename {R : Decided.Rewriter} {G G' : Graph} {old new : String} {σ : K → K}
    (h : Decided.rename P R G old new = some G')
    (H : P.RenameHyps R.base (nsSubst P.key P.urlHasOrigin old new) σ G)
    {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R.base σ G b) :
    P.refs G' (blockMap R.base (nsSubst P.key P.urlHasOrigin old new) G b) =
      fun C => ∃ C₀, P.refs G b C₀ ∧ C = img σ C₀ := by
  rw [Decided.rename_success_graph P h]
  exact refs_rename H hb hx

/-- T3 exact replacement, with MERGES and arbitrary new namespace prefixes.
Only real claimant keys must stay injective. The formula does not assume any
preservation of references, classes or namespace closure. -/
theorem T3_merge_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : P.MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (i : Nat) (t : Page K) :
    P.References (renameBy R.base m G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i), ∃ u ∈ P.occClosure (rwOcc Decided.covers m o).2,
        cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  references_merge_exact H hf i t

theorem T3_merge_forward {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : P.MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (i : Nat) {t : Page K}
    (hns : ∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2,
      ∃ v ∈ P.occClosure (rwOcc Decided.covers m o).2, P.key v = σ (P.key u))
    (hr : P.References G (f.path, i) t) :
    P.References (renameBy R.base m G) ((R.apply m f).path, i) (σp σ t) :=
  references_merge_forward H hf i hns hr

/-- T3: exact universe transformation when the blanket bijection/surjection
claims fail. Namespace parents and exception remnants occur in rewrittenRefKeys. -/
theorem T3_pages_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : P.MergeHyps R.base m σ G) (p : Page K) :
    p ∈ Decided.pages P (renameBy R.base m G) ↔
      (∃ q, P.real G q = true ∧ σp σ q = p) ∨
      (p = (P.refKind p.2, p.2) ∧ p.2 ∈ rewrittenPageSeedKeys P R.base m G ∧
        ¬ (∃ k, P.claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ P.aliasEdges G, σ e.2 = p.2)) := pages_merge_exact H p

/-- T6 exact replacement under merges: the post-rename inequality is essential. -/
theorem T6_merge_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {excluded : Page K} (H : P.MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    P.Link true excluded (renameBy R.base m G) (σp σ (P.claim f)) t ↔
      σp σ (P.claim f) ∈ P.pagesR true (renameBy R.base m G) ∧
      t ∈ P.pagesR true (renameBy R.base m G) ∧
      σp σ (P.claim f) ≠ t ∧ σp σ (P.claim f) ≠ excluded ∧
      ∃ b ∈ P.pageBlocks G (P.claim f), ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc Decided.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := link_merge_exact H hf t

/-- Generic T6 helper with separately supplied exclusions; public config is fixed. -/
theorem T6_favorites {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {fav fav' : Page K} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G)
    (hfav : ∀ p ∈ P.pagesR true G, σp σ p = fav' ↔ p = fav)
    {S T : Page K} (hS : S ∈ P.pagesR true G) (hT : T ∈ P.pagesR true G) :
    P.Link true fav G S T ↔ P.Link true fav' (renameBy R.base m G) (σp σ S) (σp σ T) :=
  link_rename_favorites H hx hfav hS hT

theorem T6_near_favorites {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {fav fav' : Page K} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G)
    (hfav : ∀ p ∈ P.pagesR true G, σp σ p = fav' ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ P.pagesR true G)
    (d : NearDir) (k : Nat) {T : Page K} (hT : T ∈ P.pagesR true G) :
    P.NearP true fav G d k X T ↔
      P.NearP true fav' (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  near_rename_favorites H hx hfav hX d k hT

/-- W1: conflict detection depends on the graph as a set, across BOTH kinds. -/
theorem W1 {G H : Graph} (h : SameFiles G H) (k : K) :
    (P.Conflicted G k ↔ P.Conflicted H k) ∧
    (k ∈ P.conflictKeys G ↔ k ∈ P.conflictKeys H) ∧
    (P.conflictedB G k = true ↔ P.Conflicted G k) :=
  ⟨P.conflict_sameFiles h k, P.conflictKeys_sameFiles h k, P.conflictedB_iff⟩

/-- W2: input agreement on an alias-closed region preserves local meanings.
This stronger theorem holds even without conflict-freedom assumptions. Hence
it applies in particular to a clean region of a conflicted G and ANY clean H
agreeing on the specified raw dependency region. -/
theorem W2 (hP : P.Lawful) {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement P S G H) {p : Page K} (hp : p.2 ∈ S)
    {s : String} (hs : P.key s ∈ S) :
    P.aliasClass G p.2 = P.aliasClass H p.2 ∧
    P.resolve G s = P.resolve H s ∧
    (p ∈ Decided.pages P G ↔ p ∈ Decided.pages P H) ∧
    P.pageBlocks G p = P.pageBlocks H p :=
  ⟨Isolation.class_agree P hP A hp, Isolation.resolve_agree P hP A hs,
   Isolation.pages_agree P hP hwG hwH A hp, Isolation.pageBlocks_agree P hP A hp⟩

theorem W2_queries {L : Type} (Q : QParams K L) (hP : P.Lawful)
    {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement P S G H) (Aq : Isolation.GivenAgreement P Q S G H)
    (q : Query L) (hq : ∀ l ∈ q.leaves, Isolation.Scoped P S l)
    {p : Page K} (hp : p.2 ∈ S) {b : BlockId} (hb : b ∈ P.pageBlocks G p) :
    (Node.page p ∈ Decided.den P Q G .page q ↔ Node.page p ∈ Decided.den P Q H .page q) ∧
    (Node.block b ∈ Decided.den P Q G .block q ↔ Node.block b ∈ Decided.den P Q H .block q) :=
  ⟨Isolation.query_page_agree P Q hP hwG hwH A Aq q hp,
   Isolation.query_block_agree P Q hP hwG hwH A Aq q hq hp hb⟩

theorem W2_links (hP : P.Lawful) {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement P S G H) {s t : Page K} (hs : s.2 ∈ S) (ht : t.2 ∈ S) (fav : Decided.Config) :
    Decided.Link P fav G s t ↔ Decided.Link P fav H s t :=
  Isolation.link_agree P hP hwG hwH A hs ht fav

theorem W2_computes {S : List K} {G H : Graph} :
    Isolation.agreementB P S G H = true ↔ Isolation.Agreement P S G H :=
  Isolation.agreementB_iff P

theorem conflicts_refuse {G : Graph} {old new s : String}
    (hs : s ∈ Decided.movedNames P G old)
    (hc : P.Conflicted G (P.key s) ∨ P.Conflicted G (P.key (nsSubst P.key P.urlHasOrigin old new s)))
    (R : Decided.Rewriter) : Decided.rename P R G old new = none :=
  Decided.rename_conflict_refused P hs hc R

/-- The order law disappears on clean graphs, with no restriction on the order
beyond the original linearity. -/
theorem clean_order (hP : P.Lawful) {G : Graph} (hc : P.ConflictFree G)
    (R : Decided.Rewriter) (m : String → String) :
    ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g →
      P.fileLe (R.apply m f) (R.apply m g) = P.fileLe f g :=
  P.conflictFree_order hP hc R.base m

/-- Q619 under Q8: even an ALIAS of a pre-block page makes refs non-uniform. -/
theorem Q619_refs {L : Type} (Q : QParams K L) {G : Graph} (hw : WF G)
    {s : String} {p : Page K} (hp : p ∈ Decided.pages P G) {b : BlockId}
    (hb : P.preBlockOf Q G p = some b) (hk : P.aliasClass G (P.key s) p.2) :
    ¬ P.KindUniform3 Q true G (.refs s) := P.refs_not_kindUniform3 Q hw hp hb hk

open Witnesses3 in
theorem T3_refuted : ¬ ∀ (G H : Graph) (old new : String), WF G → T.ConflictFree G →
    Decided.rename T R G old new = some H →
      ∃ σ, Witnesses2.PageBij σ (Decided.pages T G) (Decided.pages T H) := by
  intro h
  exact merge_no_bijection (h Merge Merge' "a" "b" (by unfold WF; decide +kernel)
    (T.conflictFreeB_iff.mp merge_allowed_clean.2.1) merge_allowed_clean.2.2)

open Witnesses3 in
theorem T3_surjection_refuted : ¬ ∀ (G H : Graph) (old new : String), WF G → T.ConflictFree G →
    Decided.rename T R G old new = some H →
      ∃ σ : Page String → Page String, ∀ q ∈ Decided.pages T H,
        ∃ p ∈ Decided.pages T G, σ p = q := by
  intro h
  exact cross_no_surjection (h Cross Cross' "x" "n/y" (by unfold WF; decide +kernel)
    (T.conflictFreeB_iff.mp cross_allowed.2.1) (by decide +kernel))

open Witnesses3 in
theorem T3_image_refuted : Decided.renameAllowed T AliasMerge "a" "b" = true ∧
    T.conflictFreeB AliasMerge = true ∧
    T.refs AliasMerge' ("b", 0) ≠
      (fun C => ∃ C₀, T.refs AliasMerge ("a", 0) C₀ ∧ C = img mm C₀) :=
  ⟨alias_merge_expands.1, alias_merge_expands.2.1, alias_merge_refs_not_image⟩

open Witnesses3 in
theorem T6_refuted : ¬ ∀ (G H : Graph) (old new : String) (S U : Page String), WF G →
    T.ConflictFree G → Decided.rename T R G old new = some H →
    (T.Link true Witnesses2.fav G S U ↔ T.Link true Witnesses2.fav H
      (σp (nsSubst T.key T.urlHasOrigin old new) S) (σp (nsSubst T.key T.urlHasOrigin old new) U)) := by
  intro h
  have hl := (h Merge Merge' "a" "b" (.page, "a") (.page, "b")
    (by unfold WF; decide +kernel) (T.conflictFreeB_iff.mp merge_allowed_clean.2.1)
    merge_allowed_clean.2.2).mp (T.linkB_iff.mp merge_self_link.1)
  have := T.linkB_iff.mpr hl
  change T.linkB true Witnesses2.fav Merge' (σp mm (.page, "a")) (σp mm (.page, "b")) = true at this
  rw [merge_self_link.2] at this
  cases this

/-- Headline exact rename theorem for the actual accepting operation. Map
alias edges, then take components; recompute both occurrence and file-name
namespace seeds. No reference-key injectivity/closure compatibility is assumed. -/
theorem T3_rename_exact {R : Decided.Rewriter} {G G' : Graph} {old new : String} {σ : K → K}
    (h : Decided.rename P R G old new = some G')
    (H : P.MergeHyps R.base (nsSubst P.key P.urlHasOrigin old new) σ G)
    {f : File} (hf : f ∈ G) (i : Nat) (t p : Page K) (e : K × K) :
    (e ∈ P.aliasEdges G' ↔ e ∈ Reach.mapE σ (P.aliasEdges G)) ∧
    (P.References G' ((R.apply (nsSubst P.key P.urlHasOrigin old new) f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i),
        ∃ u ∈ P.occClosure (rwOcc Decided.covers (nsSubst P.key P.urlHasOrigin old new) o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2) ∧
    (p ∈ Decided.pages P G' ↔
      (∃ q, P.real G q = true ∧ σp σ q = p) ∨
      (p = (P.refKind p.2, p.2) ∧
        p.2 ∈ rewrittenPageSeedKeys P R.base (nsSubst P.key P.urlHasOrigin old new) G ∧
        ¬ (∃ k, P.claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ P.aliasEdges G, σ e.2 = p.2))) ∧
    P.pageBlocks G' (σp σ (P.claim f)) =
      (P.pageBlocks G (P.claim f)).map (blockMap R.base (nsSubst P.key P.urlHasOrigin old new) G) := by
  rw [Decided.rename_success_graph P h]
  exact ⟨aliasEdges_merge H e, references_merge_exact H hf i t,
    pages_merge_exact H p, pageBlocks_merge_real H hf⟩

theorem rename_refusal_exact (R : Decided.Rewriter) (G : Graph) (old new : String) :
    Decided.rename P R G old new = none ↔
      Decided.renameBaseAllowed P G old new = false ∨ Decided.JournalRisk P G old new ∨
        Decided.ownChildUsed P G old new = true :=
  Decided.rename_none_iff P R G old new

theorem journal_refusal_exact (R : Decided.Rewriter) {G : Graph} {old new : String}
    (hb : Decided.renameBaseAllowed P G old new = true)
    (hc : Decided.ownChildUsed P G old new = false) :
    Decided.rename P R G old new = none ↔ Decided.JournalRisk P G old new :=
  Decided.rename_journal_exact P R hb hc

theorem journal_support {R : Decided.Rewriter} {G G' : Graph} {old new : String}
    (h : Decided.rename P R G old new = some G') {b : BlockId} (hb : b ∈ P.blocks G)
    {o : OccKind × String} (ho : o ∈ P.occAt G b) {k : K}
    (hk : k ∈ (P.occClosure o.2).map P.key) (hj : Decided.JournalOwns P G k) :
    k ∈ (P.occClosure (rwOcc Decided.covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key :=
  Decided.rename_success_journal_support P h hb ho hk hj

theorem favorites_config_fixed {R : Decided.Rewriter} {s t : Decided.State} {old new : String}
    (h : Decided.renameConfiguredState P R s old new = some t) : t.config = s.config :=
  Decided.rename_config_fixed P h

theorem T6_config_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : P.MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    Decided.Link P config (renameBy R.base m G) (σp σ (P.claim f)) t ↔
      σp σ (P.claim f) ∈ Decided.pages P (renameBy R.base m G) ∧
      t ∈ Decided.pages P (renameBy R.base m G) ∧
      σp σ (P.claim f) ≠ t ∧ σ (P.claim f).2 ≠ P.key config.favoritesPage ∧
      ∃ b ∈ P.pageBlocks G (P.claim f), ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc Decided.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  Decided.link_merge_config_exact H hf t

theorem T6_config_link {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G)
    (hconfig : ∀ p ∈ Decided.pages P G,
      σ p.2 = P.key config.favoritesPage ↔ p.2 = P.key config.favoritesPage)
    {S T : Page K} (hS : S ∈ Decided.pages P G) (hT : T ∈ Decided.pages P G) :
    Decided.Link P config G S T ↔
      Decided.Link P config (renameBy R.base m G) (σp σ S) (σp σ T) :=
  Decided.link_rename_config H hx hconfig hS hT

theorem T6_config_near {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : P.RenameHyps R.base m σ G)
    (hx : P.GraphExceptionFree R.base σ G)
    (hconfig : ∀ p ∈ Decided.pages P G,
      σ p.2 = P.key config.favoritesPage ↔ p.2 = P.key config.favoritesPage)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ Decided.pages P G) (d : NearDir) (k : Nat)
    {T : Page K} (hT : T ∈ Decided.pages P G) :
    Decided.NearP P config G d k X T ↔
      Decided.NearP P config (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  Decided.near_rename_config H hx hconfig hX d k hT

theorem T6_config_computes {config : Decided.Config} {G : Graph}
    (d : NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ Decided.nearL P config G d k X ↔ Decided.NearP P config G d k X T :=
  Decided.mem_nearL P d k X T

theorem T6_config_mono {config : Decided.Config} {G : Graph}
    (d : NearDir) {k k' : Nat} (hk : k ≤ k') (X : List (Page K)) (T : Page K) :
    Decided.NearP P config G d k X T → Decided.NearP P config G d k' X T :=
  Decided.near_mono P d hk X T

theorem T6_config_both {config : Decided.Config} {G : Graph}
    (k : Nat) (X : List (Page K)) (T : Page K) :
    Decided.NearP P config G .both k X T ↔
      Decided.OutP P config G k X T ∨ Decided.InP P config G k X T :=
  Decided.near_both P k X T

theorem T6_config_class {config : Decided.Config} {G : Graph} {S T T' : Page K}
    (hT : T ∈ Decided.pages P G) (hT' : T' ∈ Decided.pages P G)
    (hc : P.aliasClass G T.2 = P.aliasClass G T'.2) (hs : S = T ↔ S = T') :
    Decided.Link P config G S T ↔ Decided.Link P config G S T' :=
  link_class hT hT' hc hs

theorem T6_config_key {config : Decided.Config} {G : Graph} {S : Page K} {s t : String}
    (h : P.key s = P.key t) :
    Decided.Link P config G S (P.resolve G s) ↔ Decided.Link P config G S (P.resolve G t) :=
  link_key h

end Headlines
end CM


#print axioms CM.Headlines.T1_set
#print axioms CM.Headlines.T1_perm
#print axioms CM.Headlines.T1_key
#print axioms CM.Headlines.T1_concept_class
#print axioms CM.Headlines.T1_references
#print axioms CM.Headlines.T1_executable
#print axioms CM.Headlines.T1_pages
#print axioms CM.Headlines.T2_partition
#print axioms CM.Headlines.T2_isolated
#print axioms CM.Headlines.T2_insert
#print axioms CM.Headlines.T2_remove
#print axioms CM.Headlines.T2_decide
#print axioms CM.Headlines.T4_occ
#print axioms CM.Headlines.T4_blocks
#print axioms CM.Headlines.T4_resolve
#print axioms CM.Headlines.T4_resolve_spec
#print axioms CM.Headlines.T4_delta
#print axioms CM.Headlines.T3_covered
#print axioms CM.Headlines.T3_references
#print axioms CM.Headlines.T3_refs
#print axioms CM.Headlines.T3_pages
#print axioms CM.Headlines.T3_blocks
#print axioms CM.Headlines.T3_fixed
#print axioms CM.Headlines.T5_bool
#print axioms CM.Headlines.T5_truth_table
#print axioms CM.Headlines.T5_views
#print axioms CM.Headlines.T5_computes
#print axioms CM.Headlines.T5_leaves
#print axioms CM.Headlines.T5_uniform
#print axioms CM.Headlines.T6_link
#print axioms CM.Headlines.T6_link_onto
#print axioms CM.Headlines.T6_near
#print axioms CM.Headlines.T6_computes
#print axioms CM.Headlines.T6_class
#print axioms CM.Headlines.T6_key
#print axioms CM.Headlines.T6_mono
#print axioms CM.Headlines.T6_both
#print axioms CM.Headlines.T3_rename
#print axioms CM.Headlines.T3_merge_exact
#print axioms CM.Headlines.T3_merge_forward
#print axioms CM.Headlines.T3_pages_exact
#print axioms CM.Headlines.T6_merge_exact
#print axioms CM.Headlines.T6_favorites
#print axioms CM.Headlines.T6_near_favorites
#print axioms CM.Headlines.W1
#print axioms CM.Headlines.W2
#print axioms CM.Headlines.W2_queries
#print axioms CM.Headlines.W2_links
#print axioms CM.Headlines.W2_computes
#print axioms CM.Headlines.conflicts_refuse
#print axioms CM.Headlines.clean_order
#print axioms CM.Headlines.Q619_refs
#print axioms CM.Headlines.T3_refuted
#print axioms CM.Headlines.T3_surjection_refuted
#print axioms CM.Headlines.T3_image_refuted
#print axioms CM.Headlines.T6_refuted
#print axioms CM.Headlines.T3_rename_exact
#print axioms CM.Headlines.rename_refusal_exact
#print axioms CM.Headlines.journal_refusal_exact
#print axioms CM.Headlines.journal_support
#print axioms CM.Headlines.favorites_config_fixed
#print axioms CM.Headlines.T6_config_exact
#print axioms CM.Headlines.T6_config_link
#print axioms CM.Headlines.T6_config_near
#print axioms CM.Headlines.T6_config_computes
#print axioms CM.Headlines.T6_config_mono
#print axioms CM.Headlines.T6_config_both
#print axioms CM.Headlines.T6_config_class
#print axioms CM.Headlines.T6_config_key

open Lean Elab Command in
-- Walk checked types and values of the headline constants; fail on a nonstandard axiom or unsafe constant.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``CM.Headlines.T1_set,
    ``CM.Headlines.T1_perm,
    ``CM.Headlines.T1_key,
    ``CM.Headlines.T1_concept_class,
    ``CM.Headlines.T1_references,
    ``CM.Headlines.T1_executable,
    ``CM.Headlines.T1_pages,
    ``CM.Headlines.T2_partition,
    ``CM.Headlines.T2_isolated,
    ``CM.Headlines.T2_insert,
    ``CM.Headlines.T2_remove,
    ``CM.Headlines.T2_decide,
    ``CM.Headlines.T4_occ,
    ``CM.Headlines.T4_blocks,
    ``CM.Headlines.T4_resolve,
    ``CM.Headlines.T4_resolve_spec,
    ``CM.Headlines.T4_delta,
    ``CM.Headlines.T3_covered,
    ``CM.Headlines.T3_references,
    ``CM.Headlines.T3_refs,
    ``CM.Headlines.T3_pages,
    ``CM.Headlines.T3_blocks,
    ``CM.Headlines.T3_fixed,
    ``CM.Headlines.T5_bool,
    ``CM.Headlines.T5_truth_table,
    ``CM.Headlines.T5_views,
    ``CM.Headlines.T5_computes,
    ``CM.Headlines.T5_leaves,
    ``CM.Headlines.T5_uniform,
    ``CM.Headlines.T6_link,
    ``CM.Headlines.T6_link_onto,
    ``CM.Headlines.T6_near,
    ``CM.Headlines.T6_computes,
    ``CM.Headlines.T6_class,
    ``CM.Headlines.T6_key,
    ``CM.Headlines.T6_mono,
    ``CM.Headlines.T6_both,
    ``CM.Headlines.T3_rename,
    ``CM.Headlines.T3_merge_exact,
    ``CM.Headlines.T3_merge_forward,
    ``CM.Headlines.T3_pages_exact,
    ``CM.Headlines.T6_merge_exact,
    ``CM.Headlines.T6_favorites,
    ``CM.Headlines.T6_near_favorites,
    ``CM.Headlines.W1,
    ``CM.Headlines.W2,
    ``CM.Headlines.W2_queries,
    ``CM.Headlines.W2_links,
    ``CM.Headlines.W2_computes,
    ``CM.Headlines.conflicts_refuse,
    ``CM.Headlines.clean_order,
    ``CM.Headlines.Q619_refs,
    ``CM.Headlines.T3_refuted,
    ``CM.Headlines.T3_surjection_refuted,
    ``CM.Headlines.T3_image_refuted,
    ``CM.Headlines.T6_refuted,
    ``CM.Headlines.T3_rename_exact,
    ``CM.Headlines.rename_refusal_exact,
    ``CM.Headlines.journal_refusal_exact,
    ``CM.Headlines.journal_support,
    ``CM.Headlines.favorites_config_fixed,
    ``CM.Headlines.T6_config_exact,
    ``CM.Headlines.T6_config_link,
    ``CM.Headlines.T6_config_near,
    ``CM.Headlines.T6_config_computes,
    ``CM.Headlines.T6_config_mono,
    ``CM.Headlines.T6_config_both,
    ``CM.Headlines.T6_config_class,
    ``CM.Headlines.T6_config_key,
    ``CM.Witnesses3.lawful,
    ``CM.Witnesses3.keyLaws,
    ``CM.Witnesses3.open_alias_policy,
    ``CM.Witnesses3.good_clean,
    ``CM.Witnesses3.good_allowed,
    ``CM.Witnesses3.good_moves,
    ``CM.Witnesses3.good_faithful,
    ``CM.Witnesses3.good_hyps,
    ``CM.Witnesses3.good_exceptionFree,
    ``CM.Witnesses3.good_features,
    ``CM.Witnesses3.good_refs,
    ``CM.Witnesses3.good_pages,
    ``CM.Witnesses3.good_favorites,
    ``CM.Witnesses3.good_link,
    ``CM.Witnesses3.merge_allowed_clean,
    ``CM.Witnesses3.merge_hyps,
    ``CM.Witnesses3.merge_not_injective,
    ``CM.Witnesses3.merge_no_bijection,
    ``CM.Witnesses3.merge_self_link,
    ``CM.Witnesses3.merge_exact_applies,
    ``CM.Witnesses3.merge_forward_applies,
    ``CM.Witnesses3.cross_allowed,
    ``CM.Witnesses3.cross_hyps,
    ``CM.Witnesses3.cross_new_namespace,
    ``CM.Witnesses3.cross_no_surjection,
    ``CM.Witnesses3.property_repaired,
    ``CM.Witnesses3.permanent_exceptions,
    ``CM.Witnesses3.priority_exception,
    ``CM.Witnesses3.alias_repaired,
    ``CM.Witnesses3.favorite_membership_and_exclusion,
    ``CM.Witnesses3.alias_merge_expands,
    ``CM.Witnesses3.alias_merge_not_image,
    ``CM.Witnesses3.alias_merge_refs_not_image,
    ``CM.Witnesses3.alias_child_eliminated,
    ``CM.Witnesses3.rewritten_alias_can_target_real,
    ``CM.Witnesses3.journal_child,
    ``CM.Witnesses3.merge_creates_path,
    ``CM.Witnesses3.query_alias,
    ``CM.Witnesses3.q619,
    ``CM.Witnesses3.query_uniform_applies,
    ``CM.Witnesses3.bad_conflicts,
    ``CM.Witnesses3.conflict_distinguishes_files,
    ``CM.Witnesses3.agreement_computes,
    ``CM.Witnesses3.conflict_set_applies,
    ``CM.Witnesses3.bad_agreement,
    ``CM.Witnesses3.bad_pages,
    ``CM.Witnesses3.bad_region_nonempty,
    ``CM.Witnesses3.bad_given,
    ``CM.Witnesses3.bad_query,
    ``CM.Witnesses3.conflict_rename_refused,
    ``CM.Witnesses3.unrelated_conflicts_allow_transport,
    ``CM.Witnesses3.journal_conflict_refused,
    ``CM.Witnesses4.journal_child_refused,
    ``CM.Witnesses4.untouched_journal_allowed,
    ``CM.Witnesses4.journal_exceptions_allowed,
    ``CM.Witnesses4.journal_prefix_refused,
    ``CM.Witnesses4.retained_journal_prefix_allowed,
    ``CM.Witnesses4.journal_alias_policy,
    ``CM.Witnesses4.journal_nonowner_alias_ignored,
    ``CM.Witnesses4.journal_blanket_still_false,
    ``CM.Witnesses4.unreferenced_parent,
    ``CM.Witnesses4.legacy_universe_equality_false,
    ``CM.Witnesses4.journal_name_parent,
    ``CM.Witnesses4.nonowner_name_parent,
    ``CM.Witnesses4.namespace_exclusions,
    ``CM.Witnesses4.name_only_hyps,
    ``CM.Witnesses4.name_only_allowed,
    ``CM.Witnesses4.name_only_old_conditions,
    ``CM.Witnesses4.name_only_exact_applies,
    ``CM.Witnesses4.name_only_no_surjection,
    ``CM.Witnesses4.parent_good_hyps,
    ``CM.Witnesses4.parent_good_transport,
    ``CM.Witnesses4.name_parent_kind_needed,
    ``CM.Witnesses4.old_W2_agreement_insufficient,
    ``CM.Witnesses4.configured_favorites_moves,
    ``CM.Witnesses4.configured_favorites_target,
    ``CM.Witnesses4.configured_transport_applies]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "missing checked constant {n}"
    if info.isUnsafe then throwError "unsafe dependency {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "nonstandard axiom {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some v := info.value? (allowOpaque := true) then
      pending := v.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
  logInfo m!"ROUND 4 HEADLINE + WITNESS CONE: {seen.size} constants; axioms {axs.toList}"
