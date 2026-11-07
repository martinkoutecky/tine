/- Presentation projections over unchanged reference/query denotations.
The source exclusion compares keys, never alias components. -/
import Witness13

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- Optional Favorites arrangement name; a normalized empty key disables it. -/
def excludedSource (favorites : Option String) (target source : K) : Bool :=
  decide (source = target) || favorites.any (fun s =>
    decide (P.key s ≠ P.key "") && decide (source = P.key s))

/-- Existing requested file: its own key. File-less alias: the least effective
name of an active real group member (Tine equivalent_page_names_with).
With no real member the fallback key cannot exclude an actual source file. -/
def panelSelfKey (G : Graph) (t : Page K) : K :=
  if (openingFile P G t.2).isSome then t.2 else
    let names := ((activeGraph P G).filter (fun f =>
      sameCls ((readParams P G).aliasEdges (activeGraph P G)) (P.claim f).2 t.2)).map P.name
    ((minBy (fun a b : String => decide (a ≤ b)) names).map P.key).getD t.2

def panelSource (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) : Bool :=
  match fileAt (activeGraph P G) b.1 with
  | none => false
  | some f => !excludedSource P favorites (panelSelfKey P G t) (P.claim f).2

/-- Unprojected, deduplicated reference hits; alias members share this relation. -/
def relationHits (G : Graph) (t : Page K) : List BlockId :=
  (blocks P G).eraseDups.filter (fun b => referencesB P G b t)

/-- Linked References is this projection, and counts consume this same list. -/
def linked (favorites : Option String) (G : Graph) (t : Page K) : List BlockId :=
  (relationHits P G t).filter (panelSource P favorites G t)

def referenceCount (favorites : Option String) (G : Graph) (t : Page K) : Nat :=
  (linked P favorites G t).length

theorem panelSource_exact (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    panelSource P favorites G t b = true ↔
      ∃ f, fileAt (activeGraph P G) b.1 = some f ∧ (P.claim f).2 ≠ panelSelfKey P G t ∧
        ∀ s ∈ favorites, P.key s ≠ P.key "" → (P.claim f).2 ≠ P.key s := by
  cases h : fileAt (activeGraph P G) b.1 with
  | none => simp [panelSource, h]
  | some f =>
    cases favorites with
    | none => simp [panelSource, h, excludedSource]
    | some s =>
      by_cases he : P.key s = P.key "" <;> simp [panelSource, h, excludedSource, he]

theorem panelSelfKey_real (G : Graph) (t : Page K) (f : File)
    (h : openingFile P G t.2 = some f) : panelSelfKey P G t = t.2 := by
  simp [panelSelfKey, h]

theorem referenceCount_panel (favorites : Option String) (G : Graph) (t : Page K) :
    referenceCount P favorites G t = (linked P favorites G t).length := rfl

/-- Parser row kinds distinguish the page-property header and preamble prose.
Neither pre-block participates in the body parent/child tree. -/
inductive QueryRowKind | body | header | preamble
  deriving DecidableEq, Repr

structure QueryRowParser where
  kind : Format → String → Nat → QueryRowKind

structure QueryPresentation where
  removeBlockChildren : Bool := true
  showHeaders : Bool := false
  deriving Repr

def rowKind (R : QueryRowParser) (G : Graph) (b : BlockId) : QueryRowKind :=
  match fileAt (activeGraph P G) b.1 with
  | some f => R.kind f.format f.text b.2
  | none => .body

/-- The last body ancestor is the immediate parent. Only that parent matters. -/
def queryParent (Q : QParams K L) (R : QueryRowParser) (G : Graph) (b : BlockId) : Option BlockId :=
  if rowKind P R G b = .body then
    ((ancestors Q (activeGraph P G) b).filter (fun a => rowKind P R G a == .body)).getLast?
  else none

/-- Parent matching uses the entire raw hit list, before either UI filter. -/
def queryRowVisible (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : Node K → Bool
  | .page _ => true
  | .block b => (o.showHeaders || decide (rowKind P R G b ≠ .header)) &&
      !(o.removeBlockChildren && (queryParent P Q R G b).any (fun a => raw.contains (.block a)))

def queryProjection (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : List (Node K) :=
  raw.filter (queryRowVisible P Q R o G raw)

def queryResults (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  queryProjection P Q R o G (den P Q G a q)

theorem queryProjection_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) (n : Node K) :
    n ∈ queryProjection P Q R o G raw ↔ n ∈ raw ∧ queryRowVisible P Q R o G raw n = true := by
  simp [queryProjection]

theorem queryResults_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P Q R o G a q ↔
      n ∈ den P Q G a q ∧ queryRowVisible P Q R o G (den P Q G a q) n = true := by
  exact queryProjection_exact P Q R o G _ n

theorem query_block_visible_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) (b : BlockId) :
    queryRowVisible P Q R o G raw (.block b) = true ↔
      (o.showHeaders = true ∨ rowKind P R G b ≠ .header) ∧
      (o.removeBlockChildren = true → ∀ parent ∈ queryParent P Q R G b,
        Node.block parent ∉ raw) := by
  cases hp : queryParent P Q R G b <;> cases hc : o.removeBlockChildren <;>
    simp [queryRowVisible, hp, hc, Bool.and_eq_true, Bool.or_eq_true]

theorem query_block_suppressed_exact (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) (b : BlockId) :
    queryRowVisible P Q R o G raw (.block b) = false ↔
      (o.showHeaders = false ∧ rowKind P R G b = .header) ∨
      (o.removeBlockChildren = true ∧ ∃ parent ∈ queryParent P Q R G b,
        Node.block parent ∈ raw) := by
  cases hp : queryParent P Q R G b <;> cases hc : o.removeBlockChildren <;>
    cases hh : o.showHeaders <;> by_cases hk : rowKind P R G b = .header <;>
    simp [queryRowVisible, hp, hc, hh, hk]

theorem query_no_child_filter (Q : QParams K L) (R : QueryRowParser)
    (headers : Bool) (G : Graph) (raw : List (Node K)) :
    queryProjection P Q R ⟨false, headers⟩ G raw = raw.filter (fun n => match n with
      | .page _ => true
      | .block b => headers || decide (rowKind P R G b ≠ .header)) := by
  unfold queryProjection
  apply List.filter_congr
  intro n _
  cases n <;> simp [queryRowVisible]

theorem query_raw_requested (Q : QParams K L) (R : QueryRowParser) (G : Graph)
    (raw : List (Node K)) : queryProjection P Q R ⟨false, true⟩ G raw = raw := by
  rw [query_no_child_filter]
  apply List.filter_eq_self.mpr
  intro n _
  cases n <;> rfl

theorem query_page_visible (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) (p : Page K) :
    Node.page p ∈ queryProjection P Q R o G raw ↔ Node.page p ∈ raw := by
  simp [queryProjection, queryRowVisible]

/-- Rendering views preserve the projected membership, not necessarily raw hits. -/
theorem view_queryResults (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) (v : View (Node K)) (n : Node K) :
    n ∈ displayed (v.apply (queryResults P Q R o G a q)) ↔ n ∈ queryResults P Q R o G a q :=
  view_displays v _ n

end Consolidated13
end CM
