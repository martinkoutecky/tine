import Reach
namespace CM

inductive Dir | journals | pages
  deriving DecidableEq, Repr

inductive Format | md | org
  deriving DecidableEq, Repr

structure File where
  path : String
  dir : Dir
  format : Format
  text : String
  deriving DecidableEq, Repr

abbrev Graph := List File

def WF (G : Graph) : Prop := (G.map File.path).Nodup

inductive Kind | journal | page
  deriving DecidableEq, Repr

abbrev Page (K : Type) := Kind × K

inductive OccKind | link | tag | pageEmbed | propertyKey | linkableValue | taskMarker | priority
  deriving DecidableEq, Repr

abbrev BlockId := String × Nat

structure Params (K : Type) where
  key : String → K
  stem : File → String
  title : Format → String → Option String
  aliasDecl : Format → String → List String
  outline : Format → String → List String
  occ : Format → String → List (OccKind × String)
  fileLe : File → File → Bool
  pageLe : Page K → Page K → Bool
  refKind : K → Kind
  kindPref : Kind

  aliasKinds : Kind → Bool := fun k => k == .page

  urlHasOrigin : String → Bool := fun _ => false

  inputNonblank : String → Bool := fun s => !s.isEmpty

def isNsName (url : String → Bool) (s : String) : Bool :=
  s.toList.contains '/' && !s.startsWith "../" && !s.startsWith "./" && !url s

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

def name (f : File) : String :=
  match f.dir with
  | .journals => P.stem f
  | .pages => (P.title f.format f.text).getD (P.stem f)

def fileAt (G : Graph) (p : String) : Option File := G.find? (fun f => decide (f.path = p))

def occAt (G : Graph) (b : BlockId) : List (OccKind × String) :=
  match fileAt G b.1 with
  | none => []
  | some f =>
    match (P.outline f.format f.text)[b.2]? with
    | none => []
    | some t => P.occ f.format t

def splitSlash : List Char → List (List Char)
  | [] => [[]]
  | c :: cs =>
    let r := splitSlash cs
    if c = '/' then [] :: r
    else match r with
      | h :: t => (c :: h) :: t
      | [] => [[c]]

def nsClosure (s : String) : List String :=
  let parts := (splitSlash s.toList).map String.ofList
  s :: ((List.range (parts.length - 1)).map (fun i => "/".intercalate (parts.take (i + 1)))).filter
    (fun t => !t.isEmpty)

def nameIsNamespace (s : String) : Bool := isNsName P.urlHasOrigin s

def occClosure (s : String) : List String := if P.nameIsNamespace s then nsClosure s else [s]

def namePrefixes (s : String) : List String := (nsClosure s).tail

def fileNsNames (f : File) : List String :=
  if P.inputNonblank f.text && P.nameIsNamespace (P.name f)
  then namePrefixes (P.name f) else []

def refNames (G : Graph) (b : BlockId) : List String := (P.occAt G b).flatMap (fun o => P.occClosure o.2)

end Params
end CM
