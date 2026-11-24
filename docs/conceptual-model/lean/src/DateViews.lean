import Date13
import Ref10
import Ref10
import Links
import Presentation13
import Decided
namespace CM
namespace Date13
open Params Consolidated13
variable {K L : Type} [DecidableEq K] (P : Inputs K)

def treeParent (D : Dates K) (spell : K → String) (G : Graph) (t : Page K) : Option (Page K) :=
  if t ∈ pages P D G then
    (Ref10.nsParent P.toParams (spell t.2)).bind (fun s =>
      (pages P D G).find? (fun p => decide (p = target P D G s)))
  else none

def children (D : Dates K) (spell : K → String) (G : Graph) (t : Page K) : List (Page K) :=
  (pages P D G).eraseDups.filter (fun c => decide (treeParent P D spell G c = some t))

def tree (D : Dates K) (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (pages P D G).eraseDups.map (fun t => (t, children P D spell G t))

def namespaceTree (D : Dates K) (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (tree P D spell G).filter (fun row => (treeParent P D spell G row.1).isSome || !row.2.isEmpty)

theorem children_exact (D : Dates K) (spell : K → String) (G : Graph) (t c : Page K) :
    c ∈ children P D spell G t ↔ c ∈ pages P D G ∧ treeParent P D spell G c = some t := by simp [children]

def panelSelf (D : Dates K) (G : Graph) (t : Page K) : Page K :=
  openTarget P D G t

def panelSource (D : Dates K) (favorites : Option String) (G : Graph)
    (t : Page K) (b : BlockId) : Bool :=
  match fileAt (activeGraph P D G) b.1 with
  | none => false
  | some f => decide (claim P D f ≠ panelSelf P D G t) &&
      !favorites.any (fun s => decide (P.key s ≠ P.key "") && decide (claim P D f = target P D G s))

def linked (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : List BlockId :=
  (blocks P D G).eraseDups.filter (fun b =>
    referencesB P D G b t && panelSource P D favorites G t b)

def referenceCount (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : Nat :=
  (linked P D favorites G t).length

theorem linked_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    b ∈ linked P D favorites G t ↔ b ∈ blocks P D G ∧
      referencesB P D G b t = true ∧ panelSource P D favorites G t b = true := by simp [linked]

theorem count_panel (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    referenceCount P D favorites G t = (linked P D favorites G t).length := rfl

theorem panelSource_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    panelSource P D favorites G t b = true ↔ ∃ f,
      fileAt (activeGraph P D G) b.1 = some f ∧ claim P D f ≠ panelSelf P D G t ∧
      ∀ s ∈ favorites, P.key s ≠ P.key "" → claim P D f ≠ target P D G s := by
  cases hf : fileAt (activeGraph P D G) b.1 with
  | none => simp [panelSource, hf]
  | some f =>
    cases favorites with
    | none => simp [panelSource, hf]
    | some s => by_cases he : P.key s = P.key "" <;> simp [panelSource, hf, he]

theorem panel_opening (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    linked P D favorites G t = linked P D favorites G (openTarget P D G t) := by
  unfold linked
  apply List.filter_congr
  intro b _
  rw [references_opening]
  simp only [panelSource,panelSelf,opening_idempotent]
  rfl

def linkedAt (D : Dates K) (favorites : Option String) (G : Graph) (s : String) : List BlockId :=
  linked P D favorites G (target P D G s)

theorem name_panel_is_opening_panel (D : Dates K) (favorites : Option String) (G : Graph) (s : String) :
    linkedAt P D favorites G s = linked P D favorites G (resolve P D G s) :=
  panel_opening P D favorites G _


def leafPage (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → Page K → Bool
  | .refs _, _ => false
  | .name s, t => decide (target P D G s = t)
  | .page l, t => leafPage D Q G l t
  | .given l, t => Q.pageLeaf l (activeGraph P D G) t

def pathTargets (D : Dates K) (Q : QParams K L) (G : Graph) (b : BlockId) : List (Page K) :=
  refTargets P D G b ++
    (ancestors Q (activeGraph P D G) b).flatMap (refTargets P D G) ++
    ((pageOfBlock P D G b).toList)

def leafBlock (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => (pathTargets P D Q G b).any (fun t => sameCls (edges P D G) (target P D G s) t)
  | .name _, _ => false
  | .page l, b => ((pageOfBlock P D G b).map (leafPage P D Q G l)).getD false
  | .given l, b => Q.blockLeaf l (activeGraph P D G) b

def leafAt (D : Dates K) (Q : QParams K L) (G : Graph) (l : Leaf L) : Node K → Bool
  | .page t => leafPage P D Q G l t
  | .block b => leafBlock P D Q G l b

def univ (D : Dates K) (G : Graph) : Anchor → List (Node K)
  | .page => (pages P D G).map Node.page
  | .block => (blocks P D G).map Node.block

def den (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  (univ P D G a).filter (fun n => q.bval (fun l => leafAt P D Q G l n))

def rowKind (D : Dates K) (R : QueryRowParser) (G : Graph) (b : BlockId) : QueryRowKind :=
  match fileAt (activeGraph P D G) b.1 with
  | some f => R.kind f.format f.text b.2
  | none => .body

def queryParent (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (G : Graph) (b : BlockId) : Option BlockId :=
  if rowKind P D R G b = .body then
    ((ancestors Q (activeGraph P D G) b).filter (fun a => rowKind P D R G a == .body)).getLast?
  else none

def queryRowVisible (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : Node K → Bool
  | .page _ => true
  | .block b => (o.showHeaders || decide (rowKind P D R G b ≠ .header)) &&
      !(o.removeBlockChildren && (queryParent P D Q R G b).any (fun a => raw.contains (.block a)))

def queryProjection (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : List (Node K) :=
  raw.filter (queryRowVisible P D Q R o G raw)

def queryResults (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  queryProjection P D Q R o G (den P D Q G a q)

def links (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) : Bool :=
  decide (S ∈ pages P D G) && decide (T ∈ pages P D G) && decide (S ≠ T) &&
    decide (S ≠ target P D G config.favoritesPage) &&
    (blocks P D G).any (fun b =>
      decide (pageOfBlock P D G b = some S) && referencesB P D G b T)

def near (D : Dates K) (config : Decided.Config) (G : Graph) : NearDir → Nat → List (Page K) → List (Page K)
  | .out, n, X => walk (pages P D G) (links P D config G) n X
  | .inn, n, X => walk (pages P D G) (fun S T => links P D config G T S) n X
  | .both, n, X => walk (pages P D G) (links P D config G) n X ++
      walk (pages P D G) (fun S T => links P D config G T S) n X

theorem den_exact (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ den P D Q G a q ↔ n ∈ univ P D G a ∧ q.bval (fun l => leafAt P D Q G l n) = true := by simp [den]

theorem queryResults_exact (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P D Q R o G a q ↔ n ∈ den P D Q G a q ∧
      queryRowVisible P D Q R o G (den P D Q G a q) n = true := by simp [queryResults, queryProjection]

theorem links_exact (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) :
    links P D config G S T = true ↔ S ∈ pages P D G ∧ T ∈ pages P D G ∧ S ≠ T ∧
      S ≠ target P D G config.favoritesPage ∧ ∃ b ∈ blocks P D G,
        pageOfBlock P D G b = some S ∧ referencesB P D G b T = true := by
  simp [links, Bool.and_eq_true, List.any_eq_true, and_assoc]


theorem length_partition {α : Type} (xs : List α) (keep : α → Bool) :
    xs.length = (xs.filter keep).length + (xs.filter (fun x => !keep x)).length := by
  induction xs with
  | nil => rfl
  | cons x xs ih => cases hx : keep x <;> simp [hx,ih,Nat.add_comm,Nat.add_left_comm,Nat.add_assoc]

theorem after_count_partition (D : Dates K) (favoritesBefore favoritesAfter : Option String)
    (G H : Graph) (t : Page K) :
    referenceCount P D favoritesAfter H t =
      ((linked P D favoritesAfter H t).filter (fun b => (linked P D favoritesBefore G t).contains b)).length +
      ((linked P D favoritesAfter H t).filter (fun b => !(linked P D favoritesBefore G t).contains b)).length :=
  length_partition _ _

end Date13
end CM
