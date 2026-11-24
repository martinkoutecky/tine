import CutObserve
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

def eligible (G : Graph) (s : String) : Bool :=
  decide (Date13.target P D G s ∈ Date13.pages P D G) ||
    !(Date13.declarers P D G (Date13.target P D G s)).isEmpty

/-- Capture concerns another owner's claim. The moving source's own aliases
already belong to the moving page, as required by O3's own-alias outcome. -/
def claimedExcept (G : Graph) (except : Option File) (s : String) : Bool :=
  !((Date13.claimants P D G (Date13.target P D G s)).filter (fun f => some f != except)).isEmpty ||
    !((Date13.declarers P D G (Date13.target P D G s)).filter (fun f => some f != except)).isEmpty

def claimed (G : Graph) (s : String) : Bool := claimedExcept P D G none s

def sourceFile (G : Graph) (s : String) : Option File :=
  match Date13.claimants P D G (Date13.target P D G s) with
  | [f] => some f
  | _ => none

def allNames (G : Graph) : List String :=
  G.flatMap (fun f => P.name f :: P.fileNsNames f ++ P.aliasDecl f.format f.text ++
    (P.outline f.format f.text).flatMap (fun b => (P.occ f.format b).flatMap (fun o => P.occClosure o.2)))

def touchedNames (G : Graph) (old : String) (p : Plan) : List String :=
  let names := old :: allNames P G
  let changed := names.filter (fun s => (nsMatch P.key P.urlHasOrigin old s).isSome && decide ((Date13.target P D [] s).1 ≠ .journal))
  let ownAliases := (p.source.toList.flatMap (fun f => P.aliasDecl f.format f.text)).filter (fun s => decide ((Date13.target P D [] s).1 ≠ .journal))
  let touched := old :: changed ++ ownAliases
  touched ++ touched.map (nameMap P D old p.destination)

/-- Only the source concept may land on its own alias. A child concept must
still respect that alias claim, even when the parent file declared it. -/
def captureClaimed (G : Graph) (old : String) (p : Plan) (s : String) : Bool :=
  claimedExcept P D G
    (if Date13.target P D G s = Date13.target P D G old then p.source else none)
    (nameMap P D old p.destination s)

end Cut
end CM
