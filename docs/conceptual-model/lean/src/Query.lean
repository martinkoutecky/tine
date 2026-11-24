import Pages
set_option linter.unusedSectionVars false

namespace CM

structure QParams (K : Type) (L : Type) where
  levels : Format → String → List Nat
  preBlock : Format → String → Bool
  blockLeaf : L → Graph → BlockId → Bool
  pageLeaf : L → Graph → Page K → Bool

inductive Leaf (L : Type) | refs (s : String) | name (s : String) | page (l : Leaf L) | given (l : L)
  deriving Repr

inductive Query (L : Type) | leaf (l : Leaf L) | and (q r : Query L) | or (q r : Query L) | not (q : Query L)
  deriving Repr

inductive Anchor | block | page
  deriving DecidableEq, Repr

def Query.bval {L : Type} (v : Leaf L → Bool) : Query L → Bool
  | .leaf l => v l
  | .and q r => q.bval v && r.bval v
  | .or q r => q.bval v || r.bval v
  | .not q => !q.bval v

def isAnc (lv : List Nat) (j i : Nat) : Bool :=
  decide (j < i) && (List.range (i - j)).all (fun d => decide (lv.getD j 0 < lv.getD (j + 1 + d) 0))

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K) {L : Type} (Q : QParams K L)

def ancestors (G : Graph) (b : BlockId) : List BlockId :=
  let lv := match fileAt G b.1 with | some f => Q.levels f.format f.text | none => []
  ((List.range b.2).filter (fun j => isAnc lv j b.2)).map (b.1, ·)

end Params
end CM
