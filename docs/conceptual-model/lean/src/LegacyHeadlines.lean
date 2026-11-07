/-
Headline theorems of the Tine conceptual model v0, round 1 (T1, T2, T4) and round 2 (T3 and T6 as
replacement claims, T5), restated with their exact
statements, followed by an axiom audit.

Building this module (`lake build`) prints each statement's axioms, then walks the full dependency cone
of every headline (checked types AND proof terms) and fails the build if anything other than the three
standard axioms `propext`, `Classical.choice`, `Quot.sound` occurs, or if any dependency is `unsafe`.

Reading guide. `P : Params K` bundles the given functions (key, config, parser-facing extractors) and
two orders; `P.Lawful` says the two orders are linear, the only law used. A graph is a `List File`;
`SameFiles G₁ G₂` says two lists have the same members. `DiffAt π G G'` says `G` and `G'` have the
same files at every path other than `π`. `P.Affected π G G' a` says a file at `π` (before or after)
claims a page of key `a`.
-/
import Lean
import Locality
import Pages
import Demo
import Links
import Query
import Witnesses2

namespace CM
namespace LegacyHeadlines
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

/-- T1g. The page universe `pages G` (real ∪ reference-only, SPEC §2.3/§3.2) and the node universe
`nodes G` depend only on the set of files (paths unique). -/
theorem T1_pages (hP : P.Lawful) {G₁ G₂ : Graph} (hw : WF G₂) (h : SameFiles G₁ G₂) :
    (∀ p, p ∈ P.pagesLegacyA G₁ ↔ p ∈ P.pagesLegacyA G₂) ∧ (∀ n, n ∈ P.nodesLegacyA G₁ ↔ n ∈ P.nodesLegacyA G₂) :=
  ⟨pages_sameFiles hP hw h, nodes_sameFiles hP hw h⟩

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


/-! ## Round 2. T3 (rename), replacement claim

SPEC T3 as stated is refuted (`Witnesses2`: C1 renaming onto a reference-only page, C2 across
namespaces, C3 SPEC's literal rewriting set, C4 a property-key exception, C7 a task marker, C8 a journal
sharing the moved key (O7)). C6 is not among them: it drops SPEC's `alias::` rewrite (Tine today) and
shows only that the alias law is needed.
The replacement: under `RenameHyps` (the rewriter's laws, freshness of the new keys on the keys in
play, namespace compatibility, `refKind` preserved) for `G' = renameBy R m G`. -/

section T3
variable {P} {R : Rewriter} {m : String → String} {σ : K → K} {G : Graph}

/-- T3a. Through covered (rewritten) occurrence kinds, references are carried by `σ`, with no
exception-freeness hypothesis. -/
theorem T3_covered (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G) {t : Page K}
    (ht : P.InPlay G t.2) :
    P.RefsBy R.covers G b t ↔ P.RefsBy R.covers (renameBy R m G) (blockMap R m G b) (σp σ t) :=
  refsBy_covers_rename H hb ht

/-- T3b. For an exception-free block, all its references are carried by `σ`. -/
theorem T3_references (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R σ G b) {t : Page K} (ht : P.InPlay G t.2) :
    P.References G b t ↔ P.References (renameBy R m G) (blockMap R m G b) (σp σ t) :=
  references_rename H hb hx ht

/-- T3b (SPEC's form): `refs(G', b') = σ(refs(G, b))` as a set of concepts. -/
theorem T3_refs (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R σ G b) :
    P.refs (renameBy R m G) (blockMap R m G b) = fun C => ∃ C₀, P.refs G b C₀ ∧ C = img σ C₀ :=
  refs_rename H hb hx

/-- T3c. With the whole graph exception-free, `σp σ` is a bijection `pagesR r G → pagesR r G'`,
under either reading `r` of P1. -/
theorem T3_pages (H : P.RenameHyps R m σ G) (hx : P.GraphExceptionFree R σ G) (r : Bool) :
    (∀ p ∈ P.pagesR r G, σp σ p ∈ P.pagesR r (renameBy R m G)) ∧
    (∀ q ∈ P.pagesR r (renameBy R m G), ∃ p ∈ P.pagesR r G, σp σ p = q) ∧
    (∀ p ∈ P.pagesR r G, ∀ q ∈ P.pagesR r G, σp σ p = σp σ q → p = q) :=
  ⟨fun _ hp => pagesR_rename_fwd H hx hp, fun _ hq => pagesR_rename_onto H hx hq,
   fun _ hp _ hq e => pagesR_rename_inj H hp hq e⟩

/-- T3d. The blocks of the renamed page are the images of the old page's blocks. -/
theorem T3_blocks (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) :
    P.pageBlocks (renameBy R m G) (σp σ p) = (P.pageBlocks G p).map (blockMap R m G) :=
  pageBlocks_rename H hp

/-- T3d (SPEC's form). A page that does not move keeps exactly its blocks. -/
theorem T3_fixed (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) (hfix : σ p.2 = p.2) :
    P.pageBlocks (renameBy R m G) p = P.pageBlocks G p :=
  pageBlocks_fixed H hp hfix

end T3

/-! ## Round 2. T5 (queries) -/

section T5
variable {L : Type} (Q : QParams K L)

/-- T5a. `⟦·⟧` over any universe is a Boolean homomorphism. -/
theorem T5_bool {U : List (Node K)} {G : Graph} (q r : Query L) (n : Node K) :
    (n ∈ P.denULegacy Q U G (.and q r) ↔ n ∈ P.denULegacy Q U G q ∧ n ∈ P.denULegacy Q U G r) ∧
    (n ∈ P.denULegacy Q U G (.or q r) ↔ n ∈ P.denULegacy Q U G q ∨ n ∈ P.denULegacy Q U G r) ∧
    (n ∈ P.denULegacy Q U G (.not q) ↔ n ∈ U ∧ n ∉ P.denULegacy Q U G q) :=
  ⟨P.denU_and Q n, P.denU_or Q n, P.denU_not Q n⟩

/-- T5a. Queries with the same truth table have the same denotation. -/
theorem T5_truth_table {U : List (Node K)} {G : Graph} {q q' : Query L} (h : ∀ v, q.bval v = q'.bval v) :
    P.denULegacy Q U G q = P.denULegacy Q U G q' :=
  P.denU_bool Q h

/-- T5b. Every view displays exactly `⟦q⟧`. -/
theorem T5_views (v : View (Node K)) (r : Bool) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ displayed (v.apply (P.denLegacy Q r G a q)) ↔ n ∈ P.denLegacy Q r G a q :=
  P.view_den Q v r G a q n

/-- T5c. The model computes the kind-uniform leaves. -/
theorem T5_computes {r : Bool} {G : Graph} {ls : List (Leaf L)} {l : Leaf L} :
    l ∈ P.kindUniformLeavesLegacy Q r G ls ↔ l ∈ ls ∧ P.KindUniformLegacy Q r G l :=
  P.mem_kindUniformLeaves Q

/-- T5c. Which model-defined leaves are kind-uniform. -/
theorem T5_leaves {r : Bool} {G : Graph} (hw : WF G) (l : Leaf L) (s : String) :
    P.KindUniformLegacy Q r G (.page l) ∧
    (P.KindUniformLegacy Q r G (.refs s) ↔
      ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b → P.key s ∉ P.pathRefKeys Q G b) ∧
    (P.KindUniformLegacy Q r G (.name s) ↔ ∀ p ∈ P.pagesR r G, P.preBlockOf Q G p ≠ none → P.key s ≠ p.2) :=
  ⟨P.page_kindUniform Q hw l, P.refs_kindUniform_iff Q, P.name_kindUniform_iff Q⟩

/-- T5d. A query whose leaves are all kind-uniform treats a page and its pre-block alike. -/
theorem T5_uniform {r : Bool} {G : Graph} (hw : WF G) {q : Query L}
    (hu : ∀ l ∈ q.leaves, P.KindUniformLegacy Q r G l) {p : Page K} (hp : p ∈ P.pagesR r G) {b : BlockId}
    (hb : P.preBlockOf Q G p = some b) :
    Node.page p ∈ P.denLegacy Q r G .page q ↔ Node.block b ∈ P.denLegacy Q r G .block q :=
  P.query_uniform Q hw hu hp hb

end T5

/-! ## Round 2. T6 (links), replacement claim

SPEC T6 as stated is refuted (`Witnesses2`: C4 a property-key occurrence survives, C5 renaming onto
Favorites). The replacement adds whole-graph exception-freeness and a Favorites condition. -/

section T6
variable {P} {R : Rewriter} {m : String → String} {σ : K → K}

theorem T6_link {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S T : Page K} (hS : S ∈ P.pagesR r G) (hT : T ∈ P.pagesR r G) :
    P.Link r fav G S T ↔ P.Link r fav (renameBy R m G) (σp σ S) (σp σ T) :=
  link_rename H hx hfav hS hT

theorem T6_link_onto {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S' T' : Page K} (h : P.Link r fav (renameBy R m G) S' T') :
    ∃ S ∈ P.pagesR r G, ∃ T ∈ P.pagesR r G, σp σ S = S' ∧ σp σ T = T' ∧ P.Link r fav G S T :=
  link_rename_onto H hx hfav h

theorem T6_near {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ P.pagesR r G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ P.pagesR r G) :
    P.NearP r fav G d k X T ↔ P.NearP r fav (renameBy R m G) d k (X.map (σp σ)) (σp σ T) :=
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

/-! ## Round 2. Refutations (kernel-checked counterexamples, `Witnesses2`) -/

open Witnesses2 in
/-- SPEC T3 (bijection) fails under SPEC §5's precondition, either reading of P1 (C1). -/
theorem T3_refuted (r : Bool) : ¬ ∀ (G : Graph) (old new : String), WF G →
    T.renameAllowedLegacy G old new = true → ∃ σ', PageBij σ' (T.pagesR r G) (T.pagesR r (T.rename RW G old new)) :=
  T3_spec_refuted r

open Witnesses2 in
/-- SPEC T3 (references) fails for SPEC §5's literal rewriting set, on a valid graph and an allowed
rename (C3). -/
theorem T3_refs_refuted : ¬ ∀ (G : Graph) (old new : String) (b : BlockId), WF G →
    T.renameAllowedLegacy G old new = true → b ∈ T.blocks G → T.References G b (.page, old) →
    T.References (T.renameLiteralLegacy RW G old new) (blockMap RW (T.litMapLegacy G old new) G b) (.page, new) :=
  T3_spec_refs_refuted

open Witnesses2 in
/-- SPEC T6 (rename invariance) fails: a property-key occurrence survives (C4), either reading of P1. -/
theorem T6_refuted (r : Bool) : ¬ ∀ (G : Graph) (old new : String) (S : Page String), WF G →
    T.renameAllowedLegacy G old new = true → T.Link r fav G S (.page, old) →
    T.Link r fav (T.rename RW G old new) (σp (nsSubst T.key T.urlHasOrigin old new) S) (.page, new) :=
  T6_spec_refuted r

open Witnesses2 in
/-- T6's Favorites hypothesis is needed (C5). -/
theorem T6_fav_needed : T.RenameHyps RW mC5 mC5 C5 ∧ T.GraphExceptionFree RW mC5 C5 ∧
    T.Link false fav C5 (.page, "a") (.page, "c") ∧
    ¬ T.Link false fav C5' (σp mC5 (.page, "a")) (σp mC5 (.page, "c")) :=
  T6_needs_fav

end LegacyHeadlines
end CM

#print axioms CM.LegacyHeadlines.T1_set
#print axioms CM.LegacyHeadlines.T1_perm
#print axioms CM.LegacyHeadlines.T1_key
#print axioms CM.LegacyHeadlines.T1_concept_class
#print axioms CM.LegacyHeadlines.T1_references
#print axioms CM.LegacyHeadlines.T1_executable
#print axioms CM.LegacyHeadlines.T1_pages
#print axioms CM.LegacyHeadlines.T2_partition
#print axioms CM.LegacyHeadlines.T2_isolated
#print axioms CM.LegacyHeadlines.T2_insert
#print axioms CM.LegacyHeadlines.T2_remove
#print axioms CM.LegacyHeadlines.T2_decide
#print axioms CM.LegacyHeadlines.T4_occ
#print axioms CM.LegacyHeadlines.T4_blocks
#print axioms CM.LegacyHeadlines.T4_resolve
#print axioms CM.LegacyHeadlines.T4_resolve_spec
#print axioms CM.LegacyHeadlines.T4_delta
#print axioms CM.LegacyHeadlines.T3_covered
#print axioms CM.LegacyHeadlines.T3_references
#print axioms CM.LegacyHeadlines.T3_refs
#print axioms CM.LegacyHeadlines.T3_pages
#print axioms CM.LegacyHeadlines.T3_blocks
#print axioms CM.LegacyHeadlines.T3_fixed
#print axioms CM.LegacyHeadlines.T5_bool
#print axioms CM.LegacyHeadlines.T5_truth_table
#print axioms CM.LegacyHeadlines.T5_views
#print axioms CM.LegacyHeadlines.T5_computes
#print axioms CM.LegacyHeadlines.T5_leaves
#print axioms CM.LegacyHeadlines.T5_uniform
#print axioms CM.LegacyHeadlines.T6_link
#print axioms CM.LegacyHeadlines.T6_link_onto
#print axioms CM.LegacyHeadlines.T6_near
#print axioms CM.LegacyHeadlines.T6_computes
#print axioms CM.LegacyHeadlines.T6_class
#print axioms CM.LegacyHeadlines.T6_key
#print axioms CM.LegacyHeadlines.T6_mono
#print axioms CM.LegacyHeadlines.T6_both
#print axioms CM.LegacyHeadlines.T3_refuted
#print axioms CM.LegacyHeadlines.T3_refs_refuted
#print axioms CM.LegacyHeadlines.T6_refuted
#print axioms CM.LegacyHeadlines.T6_fav_needed

open Lean Elab Command in
-- Walk checked types and values of the headline constants; fail on a nonstandard axiom or unsafe constant.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``CM.LegacyHeadlines.T1_set, ``CM.LegacyHeadlines.T1_perm, ``CM.LegacyHeadlines.T1_key,
    ``CM.LegacyHeadlines.T1_concept_class, ``CM.LegacyHeadlines.T1_references, ``CM.LegacyHeadlines.T1_executable, ``CM.LegacyHeadlines.T1_pages,
    ``CM.LegacyHeadlines.T2_partition, ``CM.LegacyHeadlines.T2_isolated, ``CM.LegacyHeadlines.T2_insert,
    ``CM.LegacyHeadlines.T2_remove, ``CM.LegacyHeadlines.T2_decide,
    ``CM.LegacyHeadlines.T4_occ, ``CM.LegacyHeadlines.T4_blocks, ``CM.LegacyHeadlines.T4_resolve,
    ``CM.LegacyHeadlines.T4_resolve_spec, ``CM.LegacyHeadlines.T4_delta,
    ``CM.LegacyHeadlines.T3_covered, ``CM.LegacyHeadlines.T3_references, ``CM.LegacyHeadlines.T3_refs, ``CM.LegacyHeadlines.T3_pages, ``CM.LegacyHeadlines.T3_blocks, ``CM.LegacyHeadlines.T3_fixed, ``CM.LegacyHeadlines.T5_bool, ``CM.LegacyHeadlines.T5_truth_table, ``CM.LegacyHeadlines.T5_views, ``CM.LegacyHeadlines.T5_computes, ``CM.LegacyHeadlines.T5_leaves, ``CM.LegacyHeadlines.T5_uniform, ``CM.LegacyHeadlines.T6_link, ``CM.LegacyHeadlines.T6_link_onto, ``CM.LegacyHeadlines.T6_near, ``CM.LegacyHeadlines.T6_computes, ``CM.LegacyHeadlines.T6_class,
    ``CM.LegacyHeadlines.T6_key, ``CM.LegacyHeadlines.T6_mono, ``CM.LegacyHeadlines.T6_both, ``CM.LegacyHeadlines.T3_refuted, ``CM.LegacyHeadlines.T3_refs_refuted, ``CM.LegacyHeadlines.T6_refuted, ``CM.LegacyHeadlines.T6_fav_needed]
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
  logInfo m!"HEADLINE CONE: {seen.size} constants; axioms {axs.toList}"
