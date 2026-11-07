/-
Queries and views (SPEC §6.1, §6.3) and T5.

**Leaves.** SPEC §6.1 gives leaf conditions as predicates on the anchor's universe (law Q1). The model
defines three leaf forms itself, following Tine's evaluator (`tine-store/src/query/eval.rs`,
`eval_block_leaf` / `eval_page`, og 69a7132e1):
* `refs s` — path references by **key** (SPEC §6.1, O6: no alias expansion). Block reading: `key s`
  is among the keys of the namespace-closed occurrences of the block, of its ancestors, or is the key of
  its page. Page reading: false (Tine: "No accepted page-row syntax reads a page's own refs or tags").
* `name s` — the page-name leaf. Page reading: `key s = key p`. Block reading: false (Tine: "Page
  attributes only ever appear under a `page` relation").
* `page l` — the `page(…)` relation: a block reads `l` on its page; a page reads `l` on itself (Tine:
  "A `page` hop from a page row is the row itself").
Every other leaf (properties, task, content, dates, …) is `given l`, with both readings given
(`QParams`). Their typing (`Graph → node → Bool`) alone does not give Q1's locality; locality of a given leaf is the
explicit interface `GivenAgreement` (round 3, `Isolation.lean`).

**Kind-uniformity** (SPEC §6.3): leaf `l` is kind-uniform on `G` when, for every page `p` with a
pre-block `b`, `l` read on `p` equals `l` read on `b`. The model computes it (`kindUniformBLegacy`, proved
correct), and proves which model-defined leaves are kind-uniform on every graph.
-/
import Rename -- Build ordering: at most two concurrent module compilers.

set_option linter.unusedSectionVars false

namespace CM

/-- Given functions for queries. `levels` gives each outline block's depth (the parser's tree);
`preBlock fmt text` says the file's first block is its pre-block (the properties-only page header);
`blockLeaf`, `pageLeaf` are the two readings of the given leaves. -/
structure QParams (K : Type) (L : Type) where
  levels : Format → String → List Nat
  preBlock : Format → String → Bool
  blockLeaf : L → Graph → BlockId → Bool
  pageLeaf : L → Graph → Page K → Bool

/-- Leaf conditions. -/
inductive Leaf (L : Type) | refs (s : String) | name (s : String) | page (l : Leaf L) | given (l : L)
  deriving Repr

/-- Query syntax (the Boolean structure over leaves). -/
inductive Query (L : Type) | leaf (l : Leaf L) | and (q r : Query L) | or (q r : Query L) | not (q : Query L)
  deriving Repr

inductive Anchor | block | page
  deriving DecidableEq, Repr

/-- Boolean evaluation of a query under an assignment of truth values to leaves. -/
def Query.bval {L : Type} (v : Leaf L → Bool) : Query L → Bool
  | .leaf l => v l
  | .and q r => q.bval v && r.bval v
  | .or q r => q.bval v || r.bval v
  | .not q => !q.bval v

def Query.leaves {L : Type} : Query L → List (Leaf L)
  | .leaf l => [l]
  | .and q r => q.leaves ++ r.leaves
  | .or q r => q.leaves ++ r.leaves
  | .not q => q.leaves

/-- `j` is an ancestor of `i` in an outline with depths `lv`: `j < i` and every block after `j` up
to `i` is deeper than `j`. -/
def isAnc (lv : List Nat) (j i : Nat) : Bool :=
  decide (j < i) && (List.range (i - j)).all (fun d => decide (lv.getD j 0 < lv.getD (j + 1 + d) 0))

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K) {L : Type} (Q : QParams K L)

/-- The page a block belongs to (the page its file claims). -/
def pageOfBlock (G : Graph) (b : BlockId) : Option (Page K) := (fileAt G b.1).map P.claim

/-- The ancestors of a block (SPEC §6.1 path references read them). -/
def ancestors (G : Graph) (b : BlockId) : List BlockId :=
  let lv := match fileAt G b.1 with | some f => Q.levels f.format f.text | none => []
  ((List.range b.2).filter (fun j => isAnc lv j b.2)).map (b.1, ·)

/-- SPEC §6.1 path references, as keys: the block's namespace-closed occurrence keys, its ancestors',
and its page's key. -/
def pathRefKeys (G : Graph) (b : BlockId) : List K :=
  (P.refNames G b).map P.key ++ (ancestors Q G b).flatMap (fun a => (P.refNames G a).map P.key) ++
    ((P.pageOfBlock G b).map (·.2)).toList

/-- The page reading of a leaf. -/
def leafPage (G : Graph) : Leaf L → Page K → Bool
  | .refs _, _ => false
  | .name s, p => decide (P.key s = p.2)
  | .page l, p => leafPage G l p
  | .given l, p => Q.pageLeaf l G p

/-- The block reading of a leaf. -/
def leafBlockLegacy (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => decide (P.key s ∈ P.pathRefKeys Q G b)
  | .name _, _ => false
  | .page l, b => match P.pageOfBlock G b with
    | some p => P.leafPage Q G l p
    | none => false
  | .given l, b => Q.blockLeaf l G b

/-- A leaf read on a node of either kind. -/
def leafAtLegacy (G : Graph) (l : Leaf L) : Node K → Bool
  | .page p => P.leafPage Q G l p
  | .block b => P.leafBlockLegacy Q G l b

def evalNodeLegacy (G : Graph) (q : Query L) (n : Node K) : Bool := q.bval (fun l => P.leafAtLegacy Q G l n)

/-- The anchor universe: `blocks G` or `pages G` (reading `r` of P1). -/
def univ (r : Bool) (G : Graph) : Anchor → List (Node K)
  | .block => (P.blocks G).map .block
  | .page => (P.pagesR r G).map .page

/-- `⟦q⟧` over an arbitrary universe `U` (so every theorem holds for both readings of P1). -/
def denULegacy (U : List (Node K)) (G : Graph) (q : Query L) : List (Node K) := U.filter (P.evalNodeLegacy Q G q)

/-- `⟦q⟧ G` for anchor `a`. -/
def denLegacy (r : Bool) (G : Graph) (a : Anchor) (q : Query L) : List (Node K) := P.denULegacy Q (P.univ r G a) G q

/-- "Pages and blocks": the pair of readings of one query text. -/
def pagesAndBlocksLegacy (r : Bool) (G : Graph) (q : Query L) : List (Node K) × List (Node K) :=
  (P.denLegacy Q r G .page q, P.denLegacy Q r G .block q)

/-! ## T5a: `⟦·⟧` is a Boolean-algebra homomorphism -/

theorem mem_denU {U : List (Node K)} {G : Graph} {q : Query L} {n : Node K} :
    n ∈ P.denULegacy Q U G q ↔ n ∈ U ∧ P.evalNodeLegacy Q G q n = true := by
  simp [denULegacy]

theorem denU_sub {U : List (Node K)} {G : Graph} {q : Query L} {n : Node K} (h : n ∈ P.denULegacy Q U G q) :
    n ∈ U := ((P.mem_denU Q).mp h).1

theorem denU_and {U : List (Node K)} {G : Graph} {q r : Query L} (n : Node K) :
    n ∈ P.denULegacy Q U G (.and q r) ↔ n ∈ P.denULegacy Q U G q ∧ n ∈ P.denULegacy Q U G r := by
  simp only [P.mem_denU Q, evalNodeLegacy, Query.bval, Bool.and_eq_true]
  constructor
  · rintro ⟨h, a, b⟩; exact ⟨⟨h, a⟩, h, b⟩
  · rintro ⟨⟨h, a⟩, _, b⟩; exact ⟨h, a, b⟩

theorem denU_or {U : List (Node K)} {G : Graph} {q r : Query L} (n : Node K) :
    n ∈ P.denULegacy Q U G (.or q r) ↔ n ∈ P.denULegacy Q U G q ∨ n ∈ P.denULegacy Q U G r := by
  simp only [P.mem_denU Q, evalNodeLegacy, Query.bval, Bool.or_eq_true]
  exact and_or_left

theorem denU_not {U : List (Node K)} {G : Graph} {q : Query L} (n : Node K) :
    n ∈ P.denULegacy Q U G (.not q) ↔ n ∈ U ∧ n ∉ P.denULegacy Q U G q := by
  simp only [P.mem_denU Q, evalNodeLegacy, Query.bval, Bool.not_eq_true']
  constructor
  · rintro ⟨h1, h2⟩; exact ⟨h1, fun ⟨_, h⟩ => by rw [h] at h2; cases h2⟩
  · rintro ⟨h1, h2⟩
    refine ⟨h1, ?_⟩
    cases e : q.bval (fun l => P.leafAtLegacy Q G l n)
    · rfl
    · exact absurd ⟨h1, e⟩ h2

/-- The denotation depends on a query only through its Boolean function of the leaves: queries equal
in the free Boolean algebra over leaves (same truth table) denote the same subset. -/
theorem denU_bool {U : List (Node K)} {G : Graph} {q q' : Query L} (h : ∀ v, q.bval v = q'.bval v) :
    P.denULegacy Q U G q = P.denULegacy Q U G q' := by
  unfold denULegacy
  apply List.filter_congr
  intro n _
  exact h _

/-! ## Views -/

structure Row (N : Type) where
  node : N
  cells : List String
  collapsed : Bool

/-- A presentation: labelled groups of rows. -/
abbrev Presentation (N : Type) := List (String × List (Row N))

def displayed {N : Type} (p : Presentation N) : List N := p.flatMap (fun g => g.2.map Row.node)

/-- The views of SPEC §6.3. -/
inductive View (N : Type)
  | list
  | table (cols : List (N → String))
  | sort (key : N → String) (v : View N)
  | group (attr : N → String)
  | board (attr : N → String)
  | collapse (c : N → Bool) (v : View N)

/-- Distinct values, first occurrence first. -/
def uniq : List String → List String
  | [] => []
  | a :: l => let r := uniq l; if a ∈ r then r else a :: r

theorem mem_uniq {a : String} : ∀ {l : List String}, a ∈ uniq l ↔ a ∈ l
  | [] => by simp [uniq]
  | b :: l => by
    have ih := @mem_uniq a l
    simp only [uniq]
    split
    · rw [ih, List.mem_cons]
      constructor
      · exact Or.inr
      · rintro (rfl | h)
        · rw [← ih]; assumption
        · exact h
    · rw [List.mem_cons, List.mem_cons, ih]

def row0 {N : Type} (n : N) : Row N := ⟨n, [], false⟩

def groupRows {N : Type} (attr : N → String) (ns : List N) : Presentation N :=
  (uniq (ns.map attr)).map (fun a => (a, (ns.filter (fun n => decide (attr n = a))).map row0))

def View.apply {N : Type} : View N → List N → Presentation N
  | .list, ns => [("", ns.map row0)]
  | .table cols, ns => [("", ns.map (fun n => ⟨n, cols.map (· n), false⟩))]
  | .sort key v, ns => v.apply (ns.mergeSort (fun a b => decide (key a ≤ key b)))
  | .group attr, ns => groupRows attr ns
  | .board attr, ns => groupRows attr ns
  | .collapse c v, ns => (v.apply ns).map (fun g => (g.1, g.2.map (fun r => { r with collapsed := c r.node })))

/-- **T5b (view invariance).** Every view displays exactly its input set. -/
theorem view_displays {N : Type} (v : View N) (ns : List N) (x : N) : x ∈ displayed (v.apply ns) ↔ x ∈ ns := by
  induction v generalizing ns with
  | list => simp [displayed, View.apply, row0]
  | table cols => simp [displayed, View.apply]
  | sort key v ih => rw [View.apply, ih]; exact (List.mergeSort_perm _ _).mem_iff
  | group attr | board attr =>
    simp only [View.apply, groupRows, displayed, List.flatMap_map, List.mem_flatMap, List.map_map,
      List.mem_map, List.mem_filter, Function.comp, row0, decide_eq_true_eq]
    constructor
    · rintro ⟨a, _, y, ⟨hy, _⟩, rfl⟩; exact hy
    · intro hx; exact ⟨attr x, mem_uniq.mpr (List.mem_map_of_mem hx), x, ⟨hx, rfl⟩, rfl⟩
  | collapse c v ih =>
    rw [← ih ns]
    have : displayed ((v.apply ns).map (fun g => (g.1, g.2.map (fun r => { r with collapsed := c r.node })))) =
        displayed (v.apply ns) := by
      unfold displayed
      rw [List.flatMap_map]
      congr 1
      funext g
      simp [List.map_map]
    simp only [View.apply]
    rw [this]

/-- **T5b for queries.** Every view of `⟦q⟧` displays exactly `⟦q⟧`. -/
theorem view_den (v : View (Node K)) (r : Bool) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ displayed (v.apply (P.denLegacy Q r G a q)) ↔ n ∈ P.denLegacy Q r G a q := view_displays v _ n

/-! ## Kind-uniformity (SPEC §6.3) -/

/-- The pre-block of page `p`: block 0 of its owner file, when the parser marks it as the pre-block. -/
def preBlockOf (G : Graph) (p : Page K) : Option BlockId :=
  match P.owner G p with
  | some f => if Q.preBlock f.format f.text && decide (0 < (P.outline f.format f.text).length)
      then some (f.path, 0) else none
  | none => none

/-- Leaf `l` is kind-uniform on `G` (reading `r`): on every page with a pre-block, its page reading and
its block reading at the pre-block agree. -/
def KindUniformLegacy (r : Bool) (G : Graph) (l : Leaf L) : Prop :=
  ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b → P.leafPage Q G l p = P.leafBlockLegacy Q G l b

/-- Executable kind-uniformity. -/
def kindUniformBLegacy (r : Bool) (G : Graph) (l : Leaf L) : Bool :=
  (P.pagesR r G).all (fun p => match P.preBlockOf Q G p with
    | some b => P.leafPage Q G l p == P.leafBlockLegacy Q G l b
    | none => true)

theorem kindUniformB_iff {r : Bool} {G : Graph} {l : Leaf L} :
    P.kindUniformBLegacy Q r G l = true ↔ P.KindUniformLegacy Q r G l := by
  unfold kindUniformBLegacy KindUniformLegacy
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
def kindUniformLeavesLegacy (r : Bool) (G : Graph) (ls : List (Leaf L)) : List (Leaf L) :=
  ls.filter (P.kindUniformBLegacy Q r G)

theorem mem_kindUniformLeaves {r : Bool} {G : Graph} {ls : List (Leaf L)} {l : Leaf L} :
    l ∈ P.kindUniformLeavesLegacy Q r G ls ↔ l ∈ ls ∧ P.KindUniformLegacy Q r G l := by
  simp only [kindUniformLeavesLegacy, List.mem_filter, P.kindUniformB_iff Q]

theorem preBlockOf_spec {G : Graph} (hw : WF G) {p : Page K} {b : BlockId} (h : P.preBlockOf Q G p = some b) :
    P.pageOfBlock G b = some p ∧ b ∈ P.blocks G := by
  unfold preBlockOf at h
  cases ho : P.owner G p with
  | none => rw [ho] at h; cases h
  | some f =>
    rw [ho] at h
    simp only at h
    split at h
    · next hc =>
      cases h
      obtain ⟨hf, hio, hcl⟩ := (P.owner_eq_some_iff).mp ho
      simp only [Bool.and_eq_true, decide_eq_true_eq] at hc
      refine ⟨?_, (P.mem_blocks).mpr ⟨f, hf, hio, rfl, hc.2⟩⟩
      unfold pageOfBlock; rw [fileAt_eq_of_mem hw hf]; simp [hcl]
    · cases h

/-- **T5c.** The `page(…)` leaf is kind-uniform on every graph, whatever it wraps. -/
theorem page_kindUniform {r : Bool} {G : Graph} (hw : WF G) (l : Leaf L) : P.KindUniformLegacy Q r G (.page l) := by
  intro p _ b hb
  simp only [leafPage, leafBlockLegacy, (P.preBlockOf_spec Q hw hb).1]

theorem key_mem_pathRefKeys {G : Graph} {b : BlockId} {p : Page K} (h : P.pageOfBlock G b = some p) :
    p.2 ∈ P.pathRefKeys Q G b := by
  simp [pathRefKeys, h]

/-- **T5c.** `refs s` is kind-uniform exactly when no pre-block has `key s` among its path references. -/
theorem refs_kindUniform_iff {r : Bool} {G : Graph} {s : String} :
    P.KindUniformLegacy Q r G (.refs s) ↔
      ∀ p ∈ P.pagesR r G, ∀ b, P.preBlockOf Q G p = some b → P.key s ∉ P.pathRefKeys Q G b := by
  unfold KindUniformLegacy
  simp only [leafPage, leafBlockLegacy]
  constructor
  · intro h p hp b hb hk
    have := h p hp b hb
    rw [decide_eq_true hk] at this; cases this
  · intro h p hp b hb
    rw [decide_eq_false (h p hp b hb)]

/-- **T5c.** In particular `refs s` is *not* kind-uniform on any graph where a page named `s` has a
pre-block: the pre-block's path references contain its own page (SPEC §6.1), the page reading is false. -/
theorem refs_not_kindUniform {r : Bool} {G : Graph} (hw : WF G) {s : String} {p : Page K}
    (hp : p ∈ P.pagesR r G) {b : BlockId} (hb : P.preBlockOf Q G p = some b) (hk : P.key s = p.2) :
    ¬ P.KindUniformLegacy Q r G (.refs s) := by
  rw [P.refs_kindUniform_iff Q]
  intro h
  exact h p hp b hb (hk ▸ P.key_mem_pathRefKeys Q (P.preBlockOf_spec Q hw hb).1)

/-- **T5c.** `name s` is kind-uniform exactly when no page of key `key s` has a pre-block. -/
theorem name_kindUniform_iff {r : Bool} {G : Graph} {s : String} :
    P.KindUniformLegacy Q r G (.name s) ↔ ∀ p ∈ P.pagesR r G, P.preBlockOf Q G p ≠ none → P.key s ≠ p.2 := by
  unfold KindUniformLegacy
  simp only [leafPage, leafBlockLegacy]
  constructor
  · intro h p hp hb hk
    cases e : P.preBlockOf Q G p with
    | none => exact hb e
    | some b => have := h p hp b e; rw [decide_eq_true hk] at this; cases this
  · intro h p hp b hb
    rw [decide_eq_false (h p hp (by rw [hb]; simp))]

/-- **T5d (coherence of a "nodes" query, D-20261018-Q619).** If every leaf of `q` is kind-uniform on
`G`, a page and its pre-block are in or out of `q`'s two readings together. -/
theorem query_uniform {r : Bool} {G : Graph} (hw : WF G) {q : Query L}
    (hu : ∀ l ∈ q.leaves, P.KindUniformLegacy Q r G l) {p : Page K} (hp : p ∈ P.pagesR r G) {b : BlockId}
    (hb : P.preBlockOf Q G p = some b) :
    Node.page p ∈ P.denLegacy Q r G .page q ↔ Node.block b ∈ P.denLegacy Q r G .block q := by
  have hev : P.evalNodeLegacy Q G q (.page p) = P.evalNodeLegacy Q G q (.block b) := by
    unfold evalNodeLegacy
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
  unfold denLegacy
  rw [P.mem_denU Q, P.mem_denU Q, hev]
  simp only [univ, List.mem_map, Node.page.injEq, Node.block.injEq, exists_eq_right]
  have hbB := (P.preBlockOf_spec Q hw hb).2
  constructor
  · intro ⟨_, h⟩; exact ⟨hbB, h⟩
  · intro ⟨_, h⟩; exact ⟨hp, h⟩

end Params
end CM
