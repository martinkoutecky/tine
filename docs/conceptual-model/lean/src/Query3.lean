/- Round 3 query semantics (Q8). Query.lean is the retained, explicitly legacy
key-only leaf semantics. Structural parser helpers, page/name readings, and views are shared.
The r parameter remains free; the public Decided API fixes r = true (Q9). -/
import Conflicts -- Build ordering: at most two concurrent module compilers.

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K) {L : Type} (Q : QParams K L)

/-- The block reading of a leaf. -/
def leafBlock3 (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => (P.pathRefKeys Q G b).any (fun k => sameCls (P.aliasEdges G) (P.key s) k)
  | .name _, _ => false
  | .page l, b => match P.pageOfBlock G b with
    | some p => P.leafPage Q G l p
    | none => false
  | .given l, b => Q.blockLeaf l G b

/-- A leaf read on a node of either kind. -/
def leafAt3 (G : Graph) (l : Leaf L) : Node K → Bool
  | .page p => P.leafPage Q G l p
  | .block b => P.leafBlock3 Q G l b

def evalNode3 (G : Graph) (q : Query L) (n : Node K) : Bool := q.bval (fun l => P.leafAt3 Q G l n)

/-- `⟦q⟧` over an arbitrary universe `U` (so every theorem holds for both readings of P1). -/
def denU3 (U : List (Node K)) (G : Graph) (q : Query L) : List (Node K) := U.filter (P.evalNode3 Q G q)

/-- `⟦q⟧ G` for anchor `a`. -/
def den3 (r : Bool) (G : Graph) (a : Anchor) (q : Query L) : List (Node K) := P.denU3 Q (P.univ r G a) G q

/-- "Pages and blocks": the pair of readings of one query text. -/
def pagesAndBlocks3 (r : Bool) (G : Graph) (q : Query L) : List (Node K) × List (Node K) :=
  (P.den3 Q r G .page q, P.den3 Q r G .block q)

/-! ## T5a: `⟦·⟧` is a Boolean-algebra homomorphism -/

theorem mem_denU3 {U : List (Node K)} {G : Graph} {q : Query L} {n : Node K} :
    n ∈ P.denU3 Q U G q ↔ n ∈ U ∧ P.evalNode3 Q G q n = true := by
  simp [denU3]

theorem denU_sub3 {U : List (Node K)} {G : Graph} {q : Query L} {n : Node K} (h : n ∈ P.denU3 Q U G q) :
    n ∈ U := ((P.mem_denU3 Q).mp h).1

theorem denU_and3 {U : List (Node K)} {G : Graph} {q r : Query L} (n : Node K) :
    n ∈ P.denU3 Q U G (.and q r) ↔ n ∈ P.denU3 Q U G q ∧ n ∈ P.denU3 Q U G r := by
  simp only [P.mem_denU3 Q, evalNode3, Query.bval, Bool.and_eq_true]
  constructor
  · rintro ⟨h, a, b⟩; exact ⟨⟨h, a⟩, h, b⟩
  · rintro ⟨⟨h, a⟩, _, b⟩; exact ⟨h, a, b⟩

theorem denU_or3 {U : List (Node K)} {G : Graph} {q r : Query L} (n : Node K) :
    n ∈ P.denU3 Q U G (.or q r) ↔ n ∈ P.denU3 Q U G q ∨ n ∈ P.denU3 Q U G r := by
  simp only [P.mem_denU3 Q, evalNode3, Query.bval, Bool.or_eq_true]
  exact and_or_left

theorem denU_not3 {U : List (Node K)} {G : Graph} {q : Query L} (n : Node K) :
    n ∈ P.denU3 Q U G (.not q) ↔ n ∈ U ∧ n ∉ P.denU3 Q U G q := by
  simp only [P.mem_denU3 Q, evalNode3, Query.bval, Bool.not_eq_true']
  constructor
  · rintro ⟨h1, h2⟩; exact ⟨h1, fun ⟨_, h⟩ => by rw [h] at h2; cases h2⟩
  · rintro ⟨h1, h2⟩
    refine ⟨h1, ?_⟩
    cases e : q.bval (fun l => P.leafAt3 Q G l n)
    · rfl
    · exact absurd ⟨h1, e⟩ h2

/-- The denotation depends on a query only through its Boolean function of the leaves: queries equal
in the free Boolean algebra over leaves (same truth table) denote the same subset. -/
theorem denU_bool3 {U : List (Node K)} {G : Graph} {q q' : Query L} (h : ∀ v, q.bval v = q'.bval v) :
    P.denU3 Q U G q = P.denU3 Q U G q' := by
  unfold denU3
  apply List.filter_congr
  intro n _
  exact h _

/-- **T5b for queries.** Every view of `⟦q⟧` displays exactly `⟦q⟧`. -/
theorem view_den3 (v : View (Node K)) (r : Bool) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ displayed (v.apply (P.den3 Q r G a q)) ↔ n ∈ P.den3 Q r G a q := view_displays v _ n

/-! ## Kind-uniformity (SPEC §6.3) -/

/-- Leaf `l` is kind-uniform on `G` (reading `r`): on every page with a pre-block, its page reading and
its block reading at the pre-block agree. -/
def KindUniform3 (r : Bool) (G : Graph) (l : Leaf L) : Prop :=
  ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b → P.leafPage Q G l p = P.leafBlock3 Q G l b

/-- Executable kind-uniformity. -/
def kindUniformB3 (r : Bool) (G : Graph) (l : Leaf L) : Bool :=
  (P.pagesR r G).all (fun p => match P.preBlockOf Q G p with
    | some b => P.leafPage Q G l p == P.leafBlock3 Q G l b
    | none => true)

theorem kindUniformB_iff3 {r : Bool} {G : Graph} {l : Leaf L} :
    P.kindUniformB3 Q r G l = true ↔ P.KindUniform3 Q r G l := by
  unfold kindUniformB3 KindUniform3
  rw [List.all_eq_true]
  constructor
  · intro h p hp b hb
    have := h p hp
    rw [hb] at this
    simpa using this
  · intro h p hp
    cases hb : P.preBlockOf Q G p with
    | none => rfl
    | some b => simpa using h p hp b hb

/-- The model's output: which of the listed leaves are kind-uniform on `G`. -/
def kindUniformLeaves3 (r : Bool) (G : Graph) (ls : List (Leaf L)) : List (Leaf L) :=
  ls.filter (P.kindUniformB3 Q r G)

theorem mem_kindUniformLeaves3 {r : Bool} {G : Graph} {ls : List (Leaf L)} {l : Leaf L} :
    l ∈ P.kindUniformLeaves3 Q r G ls ↔ l ∈ ls ∧ P.KindUniform3 Q r G l := by
  simp only [kindUniformLeaves3, List.mem_filter, P.kindUniformB_iff3 Q]

/-- **T5c.** The `page(…)` leaf is kind-uniform on every graph, whatever it wraps. -/
theorem page_kindUniform3 {r : Bool} {G : Graph} (hw : WF G) (l : Leaf L) : P.KindUniform3 Q r G (.page l) := by
  intro p _ b hb
  simp only [leafPage, leafBlock3, (P.preBlockOf_spec Q hw hb).1]

/-- Q8: refs is uniform iff no pre-block path key belongs to the queried class. -/
theorem refs_kindUniform_iff3 {r : Bool} {G : Graph} {s : String} :
    P.KindUniform3 Q r G (.refs s) ↔
      ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b →
        ∀ k ∈ P.pathRefKeys Q G b, ¬ P.aliasClass G (P.key s) k := by
  simp [KindUniform3, leafPage, leafBlock3, Bool.false_eq,
    Bool.eq_false_iff, sameCls_iff, aliasClass]

/-- With Q8 even an alias of a pre-block page makes refs non-uniform. -/
theorem refs_not_kindUniform3 {r : Bool} {G : Graph} (hw : WF G) {s : String} {p : Page K}
    (hp : p ∈ P.pagesR r G) {b : BlockId} (hb : P.preBlockOf Q G p = some b)
    (hk : P.aliasClass G (P.key s) p.2) : ¬ P.KindUniform3 Q r G (.refs s) := by
  rw [P.refs_kindUniform_iff3 Q]
  intro h
  exact h p hp b hb p.2 (P.key_mem_pathRefKeys Q (P.preBlockOf_spec Q hw hb).1) hk

/-- Q8's key-class comparison is also comparison with the resolved concept
(Q9/AL2), independently of which direct owner AL2 selects. -/
theorem refs_leaf_concept3 (G : Graph) (s : String) (b : BlockId) :
    P.leafBlock3 Q G (.refs s) b = true ↔
      ∃ k ∈ P.pathRefKeys Q G b, P.concept G s k := by
  simp [leafBlock3, List.any_eq_true, sameCls_iff, P.concept_eq_class, aliasClass]

/-- **T5c.** `name s` is kind-uniform exactly when no page of key `key s` has a pre-block. -/
theorem name_kindUniform_iff3 {r : Bool} {G : Graph} {s : String} :
    P.KindUniform3 Q r G (.name s) ↔ ∀ p ∈ P.pagesR r G, P.preBlockOf Q G p ≠ none → P.key s ≠ p.2 := by
  unfold KindUniform3
  simp only [leafPage, leafBlock3]
  constructor
  · intro h p hp hb hk
    cases e : P.preBlockOf Q G p with
    | none => exact hb e
    | some b => have := h p hp b e; rw [decide_eq_true hk] at this; cases this
  · intro h p hp b hb
    rw [decide_eq_false (h p hp (by rw [hb]; simp))]

/-- **T5d (coherence of a "nodes" query, D-20261018-Q619).** If every leaf of `q` is kind-uniform on
`G`, a page and its pre-block are in or out of `q`'s two readings together. -/
theorem query_uniform3 {r : Bool} {G : Graph} (hw : WF G) {q : Query L}
    (hu : ∀ l ∈ q.leaves, P.KindUniform3 Q r G l) {p : Page K} (hp : p ∈ P.pagesR r G) {b : BlockId}
    (hb : P.preBlockOf Q G p = some b) :
    Node.page p ∈ P.den3 Q r G .page q ↔ Node.block b ∈ P.den3 Q r G .block q := by
  have hev : P.evalNode3 Q G q (.page p) = P.evalNode3 Q G q (.block b) := by
    unfold evalNode3
    induction q with
    | leaf l => exact hu l (by simp [Query.leaves]) p hp b hb
    | and q r ihq ihr =>
      simp only [Query.bval]
      rw [ihq (fun l hl => hu l (by simp [Query.leaves, hl])), ihr (fun l hl => hu l (by simp [Query.leaves, hl]))]
    | or q r ihq ihr =>
      simp only [Query.bval]
      rw [ihq (fun l hl => hu l (by simp [Query.leaves, hl])), ihr (fun l hl => hu l (by simp [Query.leaves, hl]))]
    | not q ih =>
      simp only [Query.bval]
      rw [ih (fun l hl => hu l (by simp [Query.leaves, hl]))]
  unfold den3
  rw [P.mem_denU3 Q, P.mem_denU3 Q, hev]
  simp only [univ, List.mem_map, Node.page.injEq, Node.block.injEq, exists_eq_right]
  have hbB := (P.preBlockOf_spec Q hw hb).2
  constructor
  · intro ⟨_, h⟩; exact ⟨hbB, h⟩
  · intro ⟨_, h⟩; exact ⟨hp, h⟩

end Params
end CM
