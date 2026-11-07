/- Round-5 specialisations: exact types and hypotheses retained from Headlines.
The general suite remains unchanged; (D) below denotes the decided instance. -/
import Headlines
import Policy5
import Witnesses5

namespace CM
namespace Fold5
open CM CM.Params

variable {K : Type} [DecidableEq K] (base : Params K) (dateKey : K → Bool)
local notation "D" => Policy5.params base dateKey

/-! ## T1: resolution is well defined -/

/-- T1a. `resolve` and `concept` depend on the graph only as a set of files. -/
theorem T1_set (hP : (D).Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (s : String) :
    (D).resolve G₁ s = (D).resolve G₂ s ∧ (D).concept G₁ s = (D).concept G₂ s :=
  ⟨resolve_sameFiles hP h s, concept_sameFiles hP h s⟩

/-- T1b. In particular, not on the listing order. -/
theorem T1_perm (hP : (D).Lawful) {G₁ G₂ : Graph} (h : G₁.Perm G₂) (s : String) :
    (D).resolve G₁ s = (D).resolve G₂ s ∧ (D).concept G₁ s = (D).concept G₂ s :=
  resolve_perm hP h s

/-- T1c. They depend on a spelling only through `key`. -/
theorem T1_key (G : Graph) {s t : String} (h : (D).key s = (D).key t) :
    (D).resolve G s = (D).resolve G t ∧ (D).concept G s = (D).concept G t :=
  resolve_key G h

/-- T1d. The concept of a spelling is the alias class of its own key (whatever `resolve` picks). -/
theorem T1_concept_class (G : Graph) (s : String) : (D).concept G s = (D).aliasClass G ((D).key s) :=
  (D).concept_eq_class G s

/-- T1e. The Linked References relation is a function of the set of files (paths unique). -/
theorem T1_references (hP : (D).Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂)
    (b : BlockId) (t : Page K) : (D).References G₁ b t ↔ (D).References G₂ b t :=
  references_sameFiles hP hw h b t

/-- T1f. The executable forms decide the propositional ones. -/
theorem T1_executable (G : Graph) (s : String) (k : K) (b : BlockId) (t : Page K) :
    ((D).conceptB G s k = true ↔ (D).concept G s k) ∧ ((D).referencesB G b t = true ↔ (D).References G b t) :=
  ⟨(D).conceptB_iff G s k, (D).referencesB_iff G b t⟩

/-- T1g. The Q9 page and node universes depend only on the file set. -/
theorem T1_pages (hP : (D).Lawful) {G H : Graph} (hw : WF H) (h : SameFiles G H) :
    (∀ p, p ∈ Decided.pages (D) G ↔ p ∈ Decided.pages (D) H) ∧
    (∀ n, n ∈ Decided.nodes (D) G ↔ n ∈ Decided.nodes (D) H) := by
  refine ⟨Decided.pages_sameFiles (D) hP hw h, ?_⟩
  intro n
  cases n with
  | page p => simpa [Decided.nodes] using Decided.pages_sameFiles (D) hP hw h p
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
    (hb : b.1 ≠ π) : (D).occAt G b = (D).occAt G' b :=
  occAt_local hw hD b hb

/-- T4b. A block of another file stays or stays out of the block set unless that file claims a page
a touched file claims. -/
theorem T4_blocks (hP : (D).Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {b : BlockId}
    (hb : b.1 ≠ π)
    (hp : ∀ g, (g ∈ G ∨ g ∈ G') → g.path = b.1 →
      ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → (D).claim h ≠ (D).claim g) :
    b ∈ (D).blocks G ↔ b ∈ (D).blocks G' :=
  blocks_local hP hD hb hp

/-- T4c (sharp form). `resolve s` and `concept s` are unchanged when the alias class of `key s`,
before and after, contains no key claimed by the file at `π`. -/
theorem T4_resolve (hP : (D).Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (s : String)
    (h1 : ∀ y, (D).aliasClass G ((D).key s) y → ¬ (D).Affected π G G' y)
    (h2 : ∀ y, (D).aliasClass G' ((D).key s) y → ¬ (D).Affected π G G' y) :
    (D).resolve G s = (D).resolve G' s ∧ (D).concept G s = (D).concept G' s :=
  resolve_local hP hD s h1 h2

/-- T4c (SPEC §7 wording): the same with "claims or declares". -/
theorem T4_resolve_spec (hP : (D).Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (s : String)
    (h1 : ∀ y, (D).aliasClass G ((D).key s) y → ¬ (D).ClaimedOrDeclared π G G' y)
    (h2 : ∀ y, (D).aliasClass G' ((D).key s) y → ¬ (D).ClaimedOrDeclared π G G' y) :
    (D).resolve G s = (D).resolve G' s ∧ (D).concept G s = (D).concept G' s :=
  resolve_local_spec hP hD s h1 h2

/-- T4d (incremental = rebuild). The alias adjacency updated by the delta (drop the edges declared
from the affected keys; add those the new owners of the affected pages declare) has exactly the edges
of the rebuilt one, so every class computed from it is the rebuilt class. -/
theorem T4_delta (hP : (D).Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') :
    (∀ e, e ∈ deltaEdges (D) ((D).aliasEdges G) (affectedKeys (D) π G G') G' ↔ e ∈ (D).aliasEdges G') ∧
    (∀ k, cls (deltaEdges (D) ((D).aliasEdges G) (affectedKeys (D) π G G') G') k = (D).aliasClass G' k) :=
  ⟨delta_eq_rebuild hP hD, delta_class_eq_rebuild hP hD⟩



/- Fresh, namespace-compatible transport, with decided occurrence coverage. -/
section T3
variable {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}

/-- T3a. Through covered (rewritten) occurrence kinds, references are carried by `σ`, with no
exception-freeness hypothesis. -/
theorem T3_covered (H : (D).RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ (D).blocks G) {t : Page K}
    (ht : (D).InPlay G t.2) :
    (D).RefsBy Decided.covers G b t ↔ (D).RefsBy Decided.covers (renameBy R.base m G) (blockMap R.base m G b) (σp σ t) :=
  refsBy_covers_rename H hb ht

/-- T3b. For an exception-free block, all its references are carried by `σ`. -/
theorem T3_references (H : (D).RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ (D).blocks G)
    (hx : (D).ExceptionFree R.base σ G b) {t : Page K} (ht : (D).InPlay G t.2) :
    (D).References G b t ↔ (D).References (renameBy R.base m G) (blockMap R.base m G b) (σp σ t) :=
  references_rename H hb hx ht

/-- T3b (SPEC's form): `refs(G', b') = σ(refs(G, b))` as a set of concepts. -/
theorem T3_refs (H : (D).RenameHyps R.base m σ G) {b : BlockId} (hb : b ∈ (D).blocks G)
    (hx : (D).ExceptionFree R.base σ G b) :
    (D).refs (renameBy R.base m G) (blockMap R.base m G b) = fun C => ∃ C₀, (D).refs G b C₀ ∧ C = img σ C₀ :=
  refs_rename H hb hx

/-- T3c. With the whole graph exception-free, `σp σ` is a bijection `pagesR r G → pagesR r G'`,
under either reading `r` of P1. -/
theorem T3_pages (H : (D).RenameHyps R.base m σ G) (hx : (D).GraphExceptionFree R.base σ G) (r : Bool) :
    (∀ p ∈ (D).pagesR r G, σp σ p ∈ (D).pagesR r (renameBy R.base m G)) ∧
    (∀ q ∈ (D).pagesR r (renameBy R.base m G), ∃ p ∈ (D).pagesR r G, σp σ p = q) ∧
    (∀ p ∈ (D).pagesR r G, ∀ q ∈ (D).pagesR r G, σp σ p = σp σ q → p = q) :=
  ⟨fun _ hp => pagesR_rename_fwd H hx hp, fun _ hq => pagesR_rename_onto H hx hq,
   fun _ hp _ hq e => pagesR_rename_inj H hp hq e⟩

/-- T3d. The blocks of the renamed page are the images of the old page's blocks. -/
theorem T3_blocks (H : (D).RenameHyps R.base m σ G) {p : Page K} (hp : (D).InPlay G p.2) :
    (D).pageBlocks (renameBy R.base m G) (σp σ p) = ((D).pageBlocks G p).map (blockMap R.base m G) :=
  pageBlocks_rename H hp

/-- T3d (SPEC's form). A page that does not move keeps exactly its blocks. -/
theorem T3_fixed (H : (D).RenameHyps R.base m σ G) {p : Page K} (hp : (D).InPlay G p.2) (hfix : σ p.2 = p.2) :
    (D).pageBlocks (renameBy R.base m G) p = (D).pageBlocks G p :=
  pageBlocks_fixed H hp hfix

end T3

section T5
variable {L : Type} (Q : QParams K L)

/-- T5a. `⟦·⟧` over any universe is a Boolean homomorphism. -/
theorem T5_bool {U : List (Node K)} {G : Graph} (q r : Query L) (n : Node K) :
    (n ∈ (D).denU3 Q U G (.and q r) ↔ n ∈ (D).denU3 Q U G q ∧ n ∈ (D).denU3 Q U G r) ∧
    (n ∈ (D).denU3 Q U G (.or q r) ↔ n ∈ (D).denU3 Q U G q ∨ n ∈ (D).denU3 Q U G r) ∧
    (n ∈ (D).denU3 Q U G (.not q) ↔ n ∈ U ∧ n ∉ (D).denU3 Q U G q) :=
  ⟨(D).denU_and3 Q n, (D).denU_or3 Q n, (D).denU_not3 Q n⟩

/-- T5a. Queries with the same truth table have the same denotation. -/
theorem T5_truth_table {U : List (Node K)} {G : Graph} {q q' : Query L} (h : ∀ v, q.bval v = q'.bval v) :
    (D).denU3 Q U G q = (D).denU3 Q U G q' :=
  (D).denU_bool3 Q h

/-- T5b. Every view displays exactly `⟦q⟧`. -/
theorem T5_views (v : View (Node K)) (r : Bool) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ displayed (v.apply ((D).den3 Q r G a q)) ↔ n ∈ (D).den3 Q r G a q :=
  (D).view_den3 Q v r G a q n

/-- T5c. The model computes the kind-uniform leaves. -/
theorem T5_computes {r : Bool} {G : Graph} {ls : List (Leaf L)} {l : Leaf L} :
    l ∈ (D).kindUniformLeaves3 Q r G ls ↔ l ∈ ls ∧ (D).KindUniform3 Q r G l :=
  (D).mem_kindUniformLeaves3 Q

/-- T5c. Which model-defined leaves are kind-uniform. -/
theorem T5_leaves {r : Bool} {G : Graph} (hw : WF G) (l : Leaf L) (s : String) :
    (D).KindUniform3 Q r G (.page l) ∧
    ((D).KindUniform3 Q r G (.refs s) ↔
      ∀ p ∈ (D).pagesR r G, ∀ b, (D).preBlockOf Q G p = some b → ∀ k ∈ (D).pathRefKeys Q G b, ¬ (D).aliasClass G ((D).key s) k) ∧
    ((D).KindUniform3 Q r G (.name s) ↔ ∀ p ∈ (D).pagesR r G, (D).preBlockOf Q G p ≠ none → (D).key s ≠ p.2) :=
  ⟨(D).page_kindUniform3 Q hw l, (D).refs_kindUniform_iff3 Q, (D).name_kindUniform_iff3 Q⟩

/-- T5d. A query whose leaves are all kind-uniform treats a page and its pre-block alike. -/
theorem T5_uniform {r : Bool} {G : Graph} (hw : WF G) {q : Query L}
    (hu : ∀ l ∈ q.leaves, (D).KindUniform3 Q r G l) {p : Page K} (hp : p ∈ (D).pagesR r G) {b : BlockId}
    (hb : (D).preBlockOf Q G p = some b) :
    Node.page p ∈ (D).den3 Q r G .page q ↔ Node.block b ∈ (D).den3 Q r G .block q :=
  (D).query_uniform3 Q hw hu hp hb

end T5

section T6
variable {R : Decided.Rewriter} {m : String → String} {σ : K → K}

theorem T6_link {r : Bool} {fav : Page K} {G : Graph} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ (D).pagesR r G, σp σ p = fav ↔ p = fav)
    {S T : Page K} (hS : S ∈ (D).pagesR r G) (hT : T ∈ (D).pagesR r G) :
    (D).Link r fav G S T ↔ (D).Link r fav (renameBy R.base m G) (σp σ S) (σp σ T) :=
  link_rename H hx hfav hS hT

theorem T6_link_onto {r : Bool} {fav : Page K} {G : Graph} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ (D).pagesR r G, σp σ p = fav ↔ p = fav)
    {S' T' : Page K} (h : (D).Link r fav (renameBy R.base m G) S' T') :
    ∃ S ∈ (D).pagesR r G, ∃ T ∈ (D).pagesR r G, σp σ S = S' ∧ σp σ T = T' ∧ (D).Link r fav G S T :=
  link_rename_onto H hx hfav h

theorem T6_near {r : Bool} {fav : Page K} {G : Graph} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G) (hfav : ∀ p ∈ (D).pagesR r G, σp σ p = fav ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ (D).pagesR r G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ (D).pagesR r G) :
    (D).NearP r fav G d k X T ↔ (D).NearP r fav (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  near_rename H hx hfav hX d k hT

/-- T6 (executable). The neighbourhoods are computed. -/
theorem T6_computes {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ (D).nearL r fav G d k X ↔ (D).NearP r fav G d k X T :=
  mem_nearL d k X T

/-- T6. A link to one alias-class member is a link to any other (same class, distinct from the source). -/
theorem T6_class {r : Bool} {fav : Page K} {G : Graph} {S T T' : Page K} (hT : T ∈ (D).pagesR r G)
    (hT' : T' ∈ (D).pagesR r G) (hc : (D).aliasClass G T.2 = (D).aliasClass G T'.2) (hs : S = T ↔ S = T') :
    (D).Link r fav G S T ↔ (D).Link r fav G S T' :=
  link_class hT hT' hc hs

end T6


section T6more

/-- T6. `link` depends on the spelling naming `T` only through `key`. -/
theorem T6_key {r : Bool} {fav : Page K} {G : Graph} {S : Page K} {s t : String} (h : (D).key s = (D).key t) :
    (D).Link r fav G S ((D).resolve G s) ↔ (D).Link r fav G S ((D).resolve G t) :=
  link_key h

/-- T6. `near` is monotone in `k`. -/
theorem T6_mono {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) {k k' : Nat} (hk : k ≤ k')
    (X : List (Page K)) (T : Page K) : (D).NearP r fav G d k X T → (D).NearP r fav G d k' X T :=
  near_mono d hk X T

/-- T6. `near X k both = out k X ∪ in k X` (by definition; `Witnesses2.GL3_near_ne_ball`: not the ball). -/
theorem T6_both {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    (D).NearP r fav G .both k X T ↔ (D).OutP r fav G k X T ∨ (D).InP r fav G k X T :=
  near_both k X T

end T6more

/-- T3 for an actually accepted decided rename, under the explicit fresh
transport hypotheses. Exceptions are per-block, rather than graph-global. -/
theorem T3_rename {R : Decided.Rewriter} {G G' : Graph} {old new : String} {σ : K → K}
    (h : Decided.rename (D) R G old new = some G')
    (H : (D).RenameHyps R.base (nsSubst (D).key (D).urlHasOrigin old new) σ G)
    {b : BlockId} (hb : b ∈ (D).blocks G)
    (hx : (D).ExceptionFree R.base σ G b) :
    (D).refs G' (blockMap R.base (nsSubst (D).key (D).urlHasOrigin old new) G b) =
      fun C => ∃ C₀, (D).refs G b C₀ ∧ C = img σ C₀ := by
  rw [Decided.rename_success_graph (D) h]
  exact refs_rename H hb hx

/-- T3 exact replacement, with MERGES and arbitrary new namespace prefixes.
Only real claimant keys must stay injective. The formula does not assume any
preservation of references, classes or namespace closure. -/
theorem T3_merge_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : (D).MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (i : Nat) (t : Page K) :
    (D).References (renameBy R.base m G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ (D).occAt G (f.path, i), ∃ u ∈ (D).occClosure (rwOcc Decided.covers m o).2,
        cls (Reach.mapE σ ((D).aliasEdges G)) ((D).key u) t.2 :=
  references_merge_exact H hf i t

theorem T3_merge_forward {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : (D).MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (i : Nat) {t : Page K}
    (hns : ∀ o ∈ (D).occAt G (f.path, i), ∀ u ∈ (D).occClosure o.2,
      ∃ v ∈ (D).occClosure (rwOcc Decided.covers m o).2, (D).key v = σ ((D).key u))
    (hr : (D).References G (f.path, i) t) :
    (D).References (renameBy R.base m G) ((R.apply m f).path, i) (σp σ t) :=
  references_merge_forward H hf i hns hr

/-- T3: exact universe transformation when the blanket bijection/surjection
claims fail. Namespace parents and exception remnants occur in rewrittenRefKeys. -/
theorem T3_pages_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : (D).MergeHyps R.base m σ G) (p : Page K) :
    p ∈ Decided.pages (D) (renameBy R.base m G) ↔
      (∃ q, (D).real G q = true ∧ σp σ q = p) ∨
      (p = ((D).refKind p.2, p.2) ∧ p.2 ∈ rewrittenPageSeedKeys (D) R.base m G ∧
        ¬ (∃ k, (D).claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ (D).aliasEdges G, σ e.2 = p.2)) := pages_merge_exact H p

/-- T6 exact replacement under merges: the post-rename inequality is essential. -/
theorem T6_merge_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {excluded : Page K} (H : (D).MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    (D).Link true excluded (renameBy R.base m G) (σp σ ((D).claim f)) t ↔
      σp σ ((D).claim f) ∈ (D).pagesR true (renameBy R.base m G) ∧
      t ∈ (D).pagesR true (renameBy R.base m G) ∧
      σp σ ((D).claim f) ≠ t ∧ σp σ ((D).claim f) ≠ excluded ∧
      ∃ b ∈ (D).pageBlocks G ((D).claim f), ∃ o ∈ (D).occAt G b,
        ∃ u ∈ (D).occClosure (rwOcc Decided.covers m o).2,
          cls (Reach.mapE σ ((D).aliasEdges G)) ((D).key u) t.2 := link_merge_exact H hf t

/-- Generic T6 helper with separately supplied exclusions; public config is fixed. -/
theorem T6_favorites {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {fav fav' : Page K} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G)
    (hfav : ∀ p ∈ (D).pagesR true G, σp σ p = fav' ↔ p = fav)
    {S T : Page K} (hS : S ∈ (D).pagesR true G) (hT : T ∈ (D).pagesR true G) :
    (D).Link true fav G S T ↔ (D).Link true fav' (renameBy R.base m G) (σp σ S) (σp σ T) :=
  link_rename_favorites H hx hfav hS hT

theorem T6_near_favorites {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {fav fav' : Page K} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G)
    (hfav : ∀ p ∈ (D).pagesR true G, σp σ p = fav' ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ (D).pagesR true G)
    (d : NearDir) (k : Nat) {T : Page K} (hT : T ∈ (D).pagesR true G) :
    (D).NearP true fav G d k X T ↔
      (D).NearP true fav' (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  near_rename_favorites H hx hfav hX d k hT

/-- W1: conflict detection depends on the graph as a set, across BOTH kinds. -/
theorem W1 {G H : Graph} (h : SameFiles G H) (k : K) :
    ((D).Conflicted G k ↔ (D).Conflicted H k) ∧
    (k ∈ (D).conflictKeys G ↔ k ∈ (D).conflictKeys H) ∧
    ((D).conflictedB G k = true ↔ (D).Conflicted G k) :=
  ⟨(D).conflict_sameFiles h k, (D).conflictKeys_sameFiles h k, (D).conflictedB_iff⟩

/-- W2: input agreement on an alias-closed region preserves local meanings.
This stronger theorem holds even without conflict-freedom assumptions. Hence
it applies in particular to a clean region of a conflicted G and ANY clean H
agreeing on the specified raw dependency region. -/
theorem W2 (hP : (D).Lawful) {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement (D) S G H) {p : Page K} (hp : p.2 ∈ S)
    {s : String} (hs : (D).key s ∈ S) :
    (D).aliasClass G p.2 = (D).aliasClass H p.2 ∧
    (D).resolve G s = (D).resolve H s ∧
    (p ∈ Decided.pages (D) G ↔ p ∈ Decided.pages (D) H) ∧
    (D).pageBlocks G p = (D).pageBlocks H p :=
  ⟨Isolation.class_agree (D) hP A hp, Isolation.resolve_agree (D) hP A hs,
   Isolation.pages_agree (D) hP hwG hwH A hp, Isolation.pageBlocks_agree (D) hP A hp⟩

theorem W2_queries {L : Type} (Q : QParams K L) (hP : (D).Lawful)
    {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement (D) S G H) (Aq : Isolation.GivenAgreement (D) Q S G H)
    (q : Query L) (hq : ∀ l ∈ q.leaves, Isolation.Scoped (D) S l)
    {p : Page K} (hp : p.2 ∈ S) {b : BlockId} (hb : b ∈ (D).pageBlocks G p) :
    (Node.page p ∈ Decided.den (D) Q G .page q ↔ Node.page p ∈ Decided.den (D) Q H .page q) ∧
    (Node.block b ∈ Decided.den (D) Q G .block q ↔ Node.block b ∈ Decided.den (D) Q H .block q) :=
  ⟨Isolation.query_page_agree (D) Q hP hwG hwH A Aq q hp,
   Isolation.query_block_agree (D) Q hP hwG hwH A Aq q hq hp hb⟩

theorem W2_links (hP : (D).Lawful) {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Isolation.Agreement (D) S G H) {s t : Page K} (hs : s.2 ∈ S) (ht : t.2 ∈ S) (fav : Decided.Config) :
    Decided.Link (D) fav G s t ↔ Decided.Link (D) fav H s t :=
  Isolation.link_agree (D) hP hwG hwH A hs ht fav

theorem W2_computes {S : List K} {G H : Graph} :
    Isolation.agreementB (D) S G H = true ↔ Isolation.Agreement (D) S G H :=
  Isolation.agreementB_iff (D)

theorem conflicts_refuse {G : Graph} {old new s : String}
    (hs : s ∈ Decided.movedNames (D) G old)
    (hc : (D).Conflicted G ((D).key s) ∨ (D).Conflicted G ((D).key (nsSubst (D).key (D).urlHasOrigin old new s)))
    (R : Decided.Rewriter) : Decided.rename (D) R G old new = none :=
  Decided.rename_conflict_refused (D) hs hc R

/-- The order law disappears on clean graphs, with no restriction on the order
beyond the original linearity. -/
theorem clean_order (hP : (D).Lawful) {G : Graph} (hc : (D).ConflictFree G)
    (R : Decided.Rewriter) (m : String → String) :
    ∀ f ∈ G, ∀ g ∈ G, (D).claim f = (D).claim g →
      (D).fileLe (R.apply m f) (R.apply m g) = (D).fileLe f g :=
  (D).conflictFree_order hP hc R.base m

/-- Q619 under Q8: even an ALIAS of a pre-block page makes refs non-uniform. -/
theorem Q619_refs {L : Type} (Q : QParams K L) {G : Graph} (hw : WF G)
    {s : String} {p : Page K} (hp : p ∈ Decided.pages (D) G) {b : BlockId}
    (hb : (D).preBlockOf Q G p = some b) (hk : (D).aliasClass G ((D).key s) p.2) :
    ¬ (D).KindUniform3 Q true G (.refs s) := (D).refs_not_kindUniform3 Q hw hp hb hk

/-- Headline exact rename theorem for the actual accepting operation. Map
alias edges, then take components; recompute both occurrence and file-name
namespace seeds. No reference-key injectivity/closure compatibility is assumed. -/
theorem T3_rename_exact {R : Decided.Rewriter} {G G' : Graph} {old new : String} {σ : K → K}
    (h : Decided.rename (D) R G old new = some G')
    (H : (D).MergeHyps R.base (nsSubst (D).key (D).urlHasOrigin old new) σ G)
    {f : File} (hf : f ∈ G) (i : Nat) (t p : Page K) (e : K × K) :
    (e ∈ (D).aliasEdges G' ↔ e ∈ Reach.mapE σ ((D).aliasEdges G)) ∧
    ((D).References G' ((R.apply (nsSubst (D).key (D).urlHasOrigin old new) f).path, i) t ↔
      ∃ o ∈ (D).occAt G (f.path, i),
        ∃ u ∈ (D).occClosure (rwOcc Decided.covers (nsSubst (D).key (D).urlHasOrigin old new) o).2,
          cls (Reach.mapE σ ((D).aliasEdges G)) ((D).key u) t.2) ∧
    (p ∈ Decided.pages (D) G' ↔
      (∃ q, (D).real G q = true ∧ σp σ q = p) ∨
      (p = ((D).refKind p.2, p.2) ∧
        p.2 ∈ rewrittenPageSeedKeys (D) R.base (nsSubst (D).key (D).urlHasOrigin old new) G ∧
        ¬ (∃ k, (D).claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ (D).aliasEdges G, σ e.2 = p.2))) ∧
    (D).pageBlocks G' (σp σ ((D).claim f)) =
      ((D).pageBlocks G ((D).claim f)).map (blockMap R.base (nsSubst (D).key (D).urlHasOrigin old new) G) := by
  rw [Decided.rename_success_graph (D) h]
  exact ⟨aliasEdges_merge H e, references_merge_exact H hf i t,
    pages_merge_exact H p, pageBlocks_merge_real H hf⟩

theorem rename_refusal_exact (R : Decided.Rewriter) (G : Graph) (old new : String) :
    Decided.rename (D) R G old new = none ↔
      Decided.renameBaseAllowed (D) G old new = false ∨ Decided.JournalRisk (D) G old new ∨
        Decided.ownChildUsed (D) G old new = true :=
  Decided.rename_none_iff (D) R G old new

theorem journal_refusal_exact (R : Decided.Rewriter) {G : Graph} {old new : String}
    (hb : Decided.renameBaseAllowed (D) G old new = true)
    (hc : Decided.ownChildUsed (D) G old new = false) :
    Decided.rename (D) R G old new = none ↔ Decided.JournalRisk (D) G old new :=
  Decided.rename_journal_exact (D) R hb hc

theorem journal_support {R : Decided.Rewriter} {G G' : Graph} {old new : String}
    (h : Decided.rename (D) R G old new = some G') {b : BlockId} (hb : b ∈ (D).blocks G)
    {o : OccKind × String} (ho : o ∈ (D).occAt G b) {k : K}
    (hk : k ∈ ((D).occClosure o.2).map (D).key) (hj : Decided.JournalOwns (D) G k) :
    k ∈ ((D).occClosure (rwOcc Decided.covers (nsSubst (D).key (D).urlHasOrigin old new) o).2).map (D).key :=
  Decided.rename_success_journal_support (D) h hb ho hk hj

theorem favorites_config_fixed {R : Decided.Rewriter} {s t : Decided.State} {old new : String}
    (h : Decided.renameConfiguredState (D) R s old new = some t) : t.config = s.config :=
  Decided.rename_config_fixed (D) h

theorem T6_config_exact {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : (D).MergeHyps R.base m σ G) {f : File} (hf : f ∈ G) (t : Page K) :
    Decided.Link (D) config (renameBy R.base m G) (σp σ ((D).claim f)) t ↔
      σp σ ((D).claim f) ∈ Decided.pages (D) (renameBy R.base m G) ∧
      t ∈ Decided.pages (D) (renameBy R.base m G) ∧
      σp σ ((D).claim f) ≠ t ∧ σ ((D).claim f).2 ≠ (D).key config.favoritesPage ∧
      ∃ b ∈ (D).pageBlocks G ((D).claim f), ∃ o ∈ (D).occAt G b,
        ∃ u ∈ (D).occClosure (rwOcc Decided.covers m o).2,
          cls (Reach.mapE σ ((D).aliasEdges G)) ((D).key u) t.2 :=
  Decided.link_merge_config_exact H hf t

theorem T6_config_link {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G)
    (hconfig : ∀ p ∈ Decided.pages (D) G,
      σ p.2 = (D).key config.favoritesPage ↔ p.2 = (D).key config.favoritesPage)
    {S T : Page K} (hS : S ∈ Decided.pages (D) G) (hT : T ∈ Decided.pages (D) G) :
    Decided.Link (D) config G S T ↔
      Decided.Link (D) config (renameBy R.base m G) (σp σ S) (σp σ T) :=
  Decided.link_rename_config H hx hconfig hS hT

theorem T6_config_near {R : Decided.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    {config : Decided.Config} (H : (D).RenameHyps R.base m σ G)
    (hx : (D).GraphExceptionFree R.base σ G)
    (hconfig : ∀ p ∈ Decided.pages (D) G,
      σ p.2 = (D).key config.favoritesPage ↔ p.2 = (D).key config.favoritesPage)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ Decided.pages (D) G) (d : NearDir) (k : Nat)
    {T : Page K} (hT : T ∈ Decided.pages (D) G) :
    Decided.NearP (D) config G d k X T ↔
      Decided.NearP (D) config (renameBy R.base m G) d k (X.map (σp σ)) (σp σ T) :=
  Decided.near_rename_config H hx hconfig hX d k hT

theorem T6_config_computes {config : Decided.Config} {G : Graph}
    (d : NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ Decided.nearL (D) config G d k X ↔ Decided.NearP (D) config G d k X T :=
  Decided.mem_nearL (D) d k X T

theorem T6_config_mono {config : Decided.Config} {G : Graph}
    (d : NearDir) {k k' : Nat} (hk : k ≤ k') (X : List (Page K)) (T : Page K) :
    Decided.NearP (D) config G d k X T → Decided.NearP (D) config G d k' X T :=
  Decided.near_mono (D) d hk X T

theorem T6_config_both {config : Decided.Config} {G : Graph}
    (k : Nat) (X : List (Page K)) (T : Page K) :
    Decided.NearP (D) config G .both k X T ↔
      Decided.OutP (D) config G k X T ∨ Decided.InP (D) config G k X T :=
  Decided.near_both (D) k X T

theorem T6_config_class {config : Decided.Config} {G : Graph} {S T T' : Page K}
    (hT : T ∈ Decided.pages (D) G) (hT' : T' ∈ Decided.pages (D) G)
    (hc : (D).aliasClass G T.2 = (D).aliasClass G T'.2) (hs : S = T ↔ S = T') :
    Decided.Link (D) config G S T ↔ Decided.Link (D) config G S T' :=
  link_class hT hT' hc hs

theorem T6_config_key {config : Decided.Config} {G : Graph} {S : Page K} {s t : String}
    (h : (D).key s = (D).key t) :
    Decided.Link (D) config G S ((D).resolve G s) ↔ Decided.Link (D) config G S ((D).resolve G t) :=
  link_key h

end Fold5
end CM

open Lean Elab Command in
-- Walk checked types and values of the headline constants; fail on a nonstandard axiom or unsafe constant.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``CM.Fold5.T1_set,
    ``CM.Fold5.T1_perm,
    ``CM.Fold5.T1_key,
    ``CM.Fold5.T1_concept_class,
    ``CM.Fold5.T1_references,
    ``CM.Fold5.T1_executable,
    ``CM.Fold5.T1_pages,
    ``CM.Fold5.T2_partition,
    ``CM.Fold5.T2_isolated,
    ``CM.Fold5.T2_insert,
    ``CM.Fold5.T2_remove,
    ``CM.Fold5.T2_decide,
    ``CM.Fold5.T4_occ,
    ``CM.Fold5.T4_blocks,
    ``CM.Fold5.T4_resolve,
    ``CM.Fold5.T4_resolve_spec,
    ``CM.Fold5.T4_delta,
    ``CM.Fold5.T3_covered,
    ``CM.Fold5.T3_references,
    ``CM.Fold5.T3_refs,
    ``CM.Fold5.T3_pages,
    ``CM.Fold5.T3_blocks,
    ``CM.Fold5.T3_fixed,
    ``CM.Fold5.T5_bool,
    ``CM.Fold5.T5_truth_table,
    ``CM.Fold5.T5_views,
    ``CM.Fold5.T5_computes,
    ``CM.Fold5.T5_leaves,
    ``CM.Fold5.T5_uniform,
    ``CM.Fold5.T6_link,
    ``CM.Fold5.T6_link_onto,
    ``CM.Fold5.T6_near,
    ``CM.Fold5.T6_computes,
    ``CM.Fold5.T6_class,
    ``CM.Fold5.T6_key,
    ``CM.Fold5.T6_mono,
    ``CM.Fold5.T6_both,
    ``CM.Fold5.T3_rename,
    ``CM.Fold5.T3_merge_exact,
    ``CM.Fold5.T3_merge_forward,
    ``CM.Fold5.T3_pages_exact,
    ``CM.Fold5.T6_merge_exact,
    ``CM.Fold5.T6_favorites,
    ``CM.Fold5.T6_near_favorites,
    ``CM.Fold5.W1,
    ``CM.Fold5.W2,
    ``CM.Fold5.W2_queries,
    ``CM.Fold5.W2_links,
    ``CM.Fold5.W2_computes,
    ``CM.Fold5.conflicts_refuse,
    ``CM.Fold5.clean_order,
    ``CM.Fold5.Q619_refs,
    ``CM.Fold5.T3_rename_exact,
    ``CM.Fold5.rename_refusal_exact,
    ``CM.Fold5.journal_refusal_exact,
    ``CM.Fold5.journal_support,
    ``CM.Fold5.favorites_config_fixed,
    ``CM.Fold5.T6_config_exact,
    ``CM.Fold5.T6_config_link,
    ``CM.Fold5.T6_config_near,
    ``CM.Fold5.T6_config_computes,
    ``CM.Fold5.T6_config_mono,
    ``CM.Fold5.T6_config_both,
    ``CM.Fold5.T6_config_class,
    ``CM.Fold5.T6_config_key,
    ``CM.Policy5.lawful,
    ``CM.Policy5.date_kind,
    ``CM.Policy5.ordinary_kind,
    ``CM.Policy5.aliases_enabled,
    ``CM.Policy5.kind_transport_iff,
    ``CM.Policy5.seed_kind_transport_iff,
    ``CM.Policy5.date_page_iff,
    ``CM.Policy5.real_page_wins, ``CM.Policy5.real_page_wins_undeclared,
    ``CM.Policy5.journal_owns_iff,
    ``CM.Policy5.journal_owns_pageOnly_iff,
    ``CM.Policy5.risk_date_independent,
    ``CM.Policy5.risk_O4_monotone,
    ``CM.Policy5.owner_alias_edge,
    ``CM.Policy5.owner_alias_reference,
    ``CM.Witnesses5.lawful,
    ``CM.Witnesses5.date_reference,
    ``CM.Witnesses5.date_parent,
    ``CM.Witnesses5.date_file_reachable,
    ``CM.Witnesses5.empty_real_date_wins,
    ``CM.Witnesses5.no_new_journal_priority,
    ``CM.Witnesses5.date_alias_suppressed,
    ``CM.Witnesses5.journal_alias_counts,
    ``CM.Witnesses5.O4_stricter_refusal,
    ``CM.Witnesses5.O4_relaxes_base_guard,
    ``CM.Witnesses5.direct_guard_boundaries,
    ``CM.Witnesses5.journal_blanket_refuted,
    ``CM.Witnesses5.date_kind_condition_needed,
    ``CM.Witnesses5.merge_allowed,
    ``CM.Witnesses5.T3_bijection_refuted,
    ``CM.Witnesses5.T3_surjection_refuted,
    ``CM.Witnesses5.T6_preservation_refuted,
    ``CM.Witnesses5.T3_image_refuted,
    ``CM.Witnesses5.Q619_still_nonuniform,
    ``CM.Witnesses5.W1_W2_nonempty,
    ``CM.Witnesses5.positive_hyps,
    ``CM.Witnesses5.positive_transport]
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
  logInfo m!"ROUND 5 POLICY + HEADLINE + WITNESS CONE: {seen.size} constants; axioms {axs.toList}"
