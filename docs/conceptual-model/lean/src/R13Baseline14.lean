/- Read-only comparison oracle for accepted r13 primitive inputs. These private
helpers have no route into current reading, candidate generation or execution.
Only the primitive snapshot is exported; the obsolete product API is retired. -/

import DateComparisonWitness13

namespace CM
namespace R13Baseline14
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K)

private def target (C : K → Option K) (G : Graph) (s : String) : Page K :=
  match openingFile P G (P.key s) with
  | some f => P.claim f
  | none => match C (P.key s) with
    | some k => (.journal, k)
    | none => (.page, P.key s)

/-- Full (kind,key) vertices distinguish an ordinary canonical-title page from
an absent journal. Explicit aliases can join either kind. -/
private def edges (C : K → Option K) (G : Graph) : List (Page K × Page K) :=
  (activeGraph P G).flatMap (fun f => (P.aliasDecl f.format f.text).map
    (fun s => (P.claim f, target P C G s)))

/-- Close the raw namespace spelling before interpreting each resulting name. -/
private def refTargets (C : K → Option K) (G : Graph) (b : BlockId) : List (Page K) :=
  (P.refNames (activeGraph P G) b).map (target P C G)

private def realFiles (G : Graph) (t : Page K) : Graph :=
  (activeGraph P G).filter (fun f => decide (P.claim f = t))

private def declarers (C : K → Option K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P G).filter (fun f => (P.aliasDecl f.format f.text).any
    (fun s => decide (target P C G s = t)))

/-- Direct declarers rank by raw path, independently of their kinds. -/
private def seedTargets (C : K → Option K) (G : Graph) : List (Page K) :=
  ((Consolidated13.blocks P G).flatMap (refTargets P C G)) ++
    (activeGraph P G).flatMap (fun f => (P.fileNsNames f).map (target P C G))

private def pages (C : K → Option K) (G : Graph) : List (Page K) :=
  (activeGraph P G).map P.claim ++
    (seedTargets P C G).filter (fun t => (realFiles P G t).isEmpty &&
      !(edges P C G).any (fun e => decide (e.2 = t)))

private def panelSelf (C : K → Option K) (G : Graph) (t : Page K) : Page K :=
  if !(realFiles P G t).isEmpty then t else
    ((minBy (fun f g => decide (P.name f ≤ P.name g))
      ((activeGraph P G).filter (fun f => sameCls (edges P C G) (P.claim f) t))).map P.claim).getD t

private def panelSource (C : K → Option K) (favorites : Option String) (G : Graph)
    (t : Page K) (b : BlockId) : Bool :=
  match fileAt (activeGraph P G) b.1 with
  | none => false
  | some f => decide (P.claim f ≠ panelSelf P C G t) &&
      !favorites.any (fun s => decide (P.key s ≠ P.key "") && decide ((P.claim f).2 = P.key s))

private def leafPage (C : K → Option K) (Q : QParams K L) (G : Graph) : Leaf L → Page K → Bool
  | .refs _, _ => false
  | .name s, t => decide (target P C G s = t)
  | .page l, t => leafPage C Q G l t
  | .given l, t => Q.pageLeaf l (activeGraph P G) t

private def pathTargets (C : K → Option K) (Q : QParams K L) (G : Graph) (b : BlockId) : List (Page K) :=
  refTargets P C G b ++
    (ancestors Q (activeGraph P G) b).flatMap (refTargets P C G) ++
    ((P.pageOfBlock (activeGraph P G) b).toList)

private def leafBlock (C : K → Option K) (Q : QParams K L) (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => (pathTargets P C Q G b).any (fun t => sameCls (edges P C G) (target P C G s) t)
  | .name _, _ => false
  | .page l, b => ((P.pageOfBlock (activeGraph P G) b).map (leafPage P C Q G l)).getD false
  | .given l, b => Q.blockLeaf l (activeGraph P G) b

private def leafAt (C : K → Option K) (Q : QParams K L) (G : Graph) (l : Leaf L) : Node K → Bool
  | .page t => leafPage P C Q G l t
  | .block b => leafBlock P C Q G l b

def inputs (C : K → Option K) (Q : QParams K L) (R : QueryRowParser)
    (config : Decided.Config) (favorites : Option String) (spell : K → String) (G : Graph) : Date13.ReaderInputs K L where
  realRows := (activeGraph P G).map P.claim
  blocks := Consolidated13.blocks P G
  seeds := (Consolidated13.blocks P G).flatMap (P.refNames (activeGraph P G)) ++ (activeGraph P G).flatMap P.fileNsNames
  virtualRow := fun s =>
    let t := target P C G s
    if (realFiles P G t).isEmpty && !(edges P C G).any (fun e => decide (e.2 = t)) then some t else none
  refNames := P.refNames (activeGraph P G)
  pathAnswer := fun s t => sameCls (edges P C G) (target P C G s) t
  leaf := leafAt P C Q G
  sourceEligible := panelSource P C favorites G
  blockSource := P.pageOfBlock (activeGraph P G)
  linkSource := fun S => decide (S.2 ≠ P.key config.favoritesPage)
  parentSpelling := fun t => Ref10.nsParent P.toParams (spell t.2)
  parentLookup := fun s => (pages P C G).find? (fun p => decide (p = target P C G s))
  header := fun b => decide (Consolidated13.rowKind P R G b = .header)
  parentBlock := Consolidated13.queryParent P Q R G
  initial := target P C G
  direct := fun s =>
    if !(realFiles P G (target P C G s)).isEmpty then none else
      (minBy (Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe)
        (declarers P C G (target P C G s))).map P.claim

/-- Exact comparison with the accepted r13 reader's primitive equations, at
arbitrary parser/configuration values. All differing input categories remain
explicit; no equality of a complete graph/query/tree output is a diagnostic. -/
theorem current_vs_r13 (D : Date13.Dates K) (C : K → Option K) (Q : QParams K L)
    (R : QueryRowParser) (config : Decided.Config) (favorites : Option String)
    (spell : K → String) (G : Graph) :
    Date13.primitiveDifference (Date13.readerInputs P D Q R config favorites spell G)
      (inputs P C Q R config favorites spell G) ∨
    Date13.ReaderAgreement (Date13.readerInputs P D Q R config favorites spell G)
      (inputs P C Q R config favorites spell G) := Date13.structural_comparison_exhaustive _ _

end R13Baseline14
end CM
