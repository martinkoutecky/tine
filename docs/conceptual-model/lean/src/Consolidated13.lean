/- Candidate operations, their finite guard, and exact previews. Safety is
conditional on explicit syntax/text realization, proved in the next module. -/
import Consolidated13Static

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

structure Candidate where
  destination : String
  survivor : Option File
  deriving DecidableEq, Repr

structure Engine where
  rewrite : Rulings9.QRewriter
  merger : (String → String) → Merger

def subst (old : String) (c : Candidate) :=
  nsSubst P.key P.urlHasOrigin old c.destination

def rewriter (E : Engine) (m : String → String) := E.rewrite.at m

def uniqueClaim (G : Graph) (k : K) : Option File :=
  match G.filter (fun f => decide ((P.claim f).2 = k)) with
  | [f] => some f
  | _ => none

def source (G : Graph) (old : String) : Option File :=
  (uniqueClaim P G (P.key old)).filter (fun f => kindOf f == .page)

/-- Literal names never route to an alias owner. A stationary literal file
receives source content, if there is source content. A moving own child does not. -/
def literal (G : Graph) (old new : String) : Candidate :=
  ⟨new, if (source P G old).isSome && decide (P.key old ≠ P.key new) then
    (uniqueClaim P G (P.key new)).filter (fun f =>
      decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) = (P.claim f).2))
    else none⟩

def groupKeys (G : Graph) (s : String) :=
  let E := (readParams P G).aliasEdges (activeGraph P G)
  (P.key s :: E.flatMap (fun e => [e.1, e.2])).eraseDups.filter
    (fun k => sameCls E (P.key s) k)

def groupFiles (G : Graph) (s : String) : Graph :=
  (activeGraph P G).filter (fun f => sameCls
    ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) (P.claim f).2)

/-- A selected member uses its own literal name for the ENTIRE substitution.
No special source-query map or typed-alias descendant map remains. -/
def candidates (G : Graph) (old new : String) : List Candidate :=
  (literal P G old new :: ((groupFiles P G new).filter (fun f =>
    decide ((P.claim f).2 ≠ P.key old))).map (fun f => literal P G old (P.name f))).eraseDups

def execute (E : Engine) (G : Graph) (old : String) (c : Candidate) : Graph :=
  match source P G old, c.survivor with
  | some S, some D => mergeBy (rewriter E (subst P old c)).base
      (E.merger (subst P old c)) (subst P old c) S D G
  | _, _ => renameBy (rewriter E (subst P old c)).base (subst P old c) G

def directNames (G : Graph) : List String :=
  G.flatMap (fun f => P.name f :: P.aliasDecl f.format f.text ++
    (P.outline f.format f.text).flatMap (fun b =>
      (P.occ f.format b).map Prod.snd ++
      (SP.extract f.format b).flatMap queryArgs))

def anchors (G : Graph) : List String :=
  G.flatMap (fun f => (if kindOf f = .journal then [P.name f] else []) ++
    (P.outline f.format f.text).flatMap (fun b =>
      ((P.occ f.format b).filter (fun o => !Decided.covers o.1)).map Prod.snd))

def demands (G : Graph) (extras : List String) (old : String) (c : Candidate) : List (K × K) :=
  (P.key old, P.key c.destination) ::
  ((directNames P SP G ++ extras).map (fun s => (P.key s, P.key (subst P old c s)))) ++
  (anchors P G).map (fun s => (P.key s, P.key s))

def consistent (ds : List (K × K)) : Bool :=
  ds.all (fun a => ds.all (fun b => decide (a.1 = b.1 → a.2 = b.2)))

def transport (ds : List (K × K)) (k : K) : K :=
  ((ds.find? (fun a => decide (a.1 = k))).map Prod.snd).getD k

def supported (G : Graph) (old : String) : Bool :=
  ((readParams P G).inPlayKeys (activeGraph P G)).contains (P.key old)

/-- Original and image conflicts: includes alias-only images (the merge
conflict omission is deliberately repaired). Unrelated conflicts remain local. -/
def touched (G : Graph) (extras : List String) (old : String) (c : Candidate) : List K :=
  let names := old :: c.destination ::
    ((Decided.usedNames P.toParams G ++ directNames P SP G ++ extras).filter (fun s =>
      (nsMatch P.key P.urlHasOrigin old s).isSome))
  names.flatMap (fun s => [P.key s, P.key (subst P old c s)])

def localClean (G : Graph) (extras : List String) (old : String) (c : Candidate) : Bool :=
  (touched P SP G extras old c).all (fun k => !P.conflictedB G k)

def pairReady (G : Graph) (old : String) (c : Candidate) : Bool :=
  match c.survivor with
  | none => true
  | some D => match source P G old with
    | none => false
    | some S => decide (S ≠ D ∧ S.format = D.format) &&
      decide (uniqueClaim P G (P.key c.destination) = some D) &&
      decide (P.key (subst P old c (P.name D)) = (P.claim D).2)

def kept (G : Graph) (old : String) (c : Candidate) : Graph :=
  if c.survivor.isSome then G.filter (fun f => decide (source P G old ≠ some f)) else G

def outputFile (E : Engine) (G : Graph) (old : String) (c : Candidate) (f : File) : File :=
  match source P G old, c.survivor with
  | some S, some D => if f = D then (E.merger (subst P old c)).apply (subst P old c) S D
      else (rewriter E (subst P old c)).apply (subst P old c) f
  | _, _ => (rewriter E (subst P old c)).apply (subst P old c) f

def ownership (E : Engine) (G : Graph) (old : String) (c : Candidate) : Bool :=
  (kept P G old c).all (fun f => (kept P G old c).all (fun g =>
    decide ((P.claim (outputFile P E G old c f)).2 =
      (P.claim (outputFile P E G old c g)).2 → (P.claim f).2 = (P.claim g).2)))

/-- Virtual page descendants may not acquire stationary content. Root joining
is explicit in the candidate; alias-only names are not pages. -/
def noCapture (G : Graph) (old : String) (c : Candidate) : Bool :=
  (Decided.movedPageNames (readParams P G) (activeGraph P G) old).all (fun s =>
    G.all (fun f =>
      decide (P.key s = P.key old) ||
      decide (P.key (subst P old c s) = P.key s) ||
      (decide (kindOf f = .page) &&
        decide (P.key (subst P old c (P.name f)) ≠ (P.claim f).2)) ||
      decide ((P.claim f).2 ≠ P.key (subst P old c s))))

def safe (E : Engine) (G : Graph) (extras : List String) (old : String) (c : Candidate) : Bool :=
  supported P G old && consistent (demands P SP G extras old c) &&
    localClean P SP G extras old c && pairReady P G old c &&
    ownership P E G old c && noCapture P G old c &&
    decide ((G.map File.path).Nodup) && decide (((execute P E G old c).map File.path).Nodup)

def offers (E : Engine) (G : Graph) (extras : List String) (old new : String) : List Candidate :=
  (candidates P G old new).filter (safe P SP E G extras old)

/-- A preference is not an authorization; an unsafe default is absent. -/
def defaultCandidate (E : Engine) (G : Graph) (extras : List String) (old new : String) : Option Candidate :=
  let c := literal P G old new
  if safe P SP E G extras old c then some c else none

def answer (E : Engine) (G : Graph) (extras : List String) (old new : String)
    (choice : Option Candidate) : Option Graph :=
  match choice with
  | none => some G
  | some c => if c ∈ offers P SP E G extras old new then some (execute P E G old c) else none

def automatic (E : Engine) (G : Graph) (extras : List String) (old new : String) : Option Graph :=
  match offers P SP E G extras old new with
  | [c] => some (execute P E G old c)
  | _ => none

/- A preview compares complete computed reference signatures, not just changed
spellings. An alias-edge change may conservatively mark extra blocks. -/
structure Signature (K : Type) where
  names : List String
  openings : List (Page K)
  edges : List (K × K)
  deriving DecidableEq

def signature (G : Graph) (b : BlockId) : Signature K :=
  let A := activeGraph P G
  let Q := readParams P G
  let names := Q.refNames A b
  ⟨names, names.map (resolve P G), Q.aliasEdges A⟩

def images (E : Engine) (G : Graph) (old : String) (c : Candidate) (b : BlockId) : List BlockId :=
  match fileAt G b.1 with
  | none => []
  | some f => match source P G old, c.survivor with
    | some S, some D => if f = S ∨ f = D then
        ((List.range (P.outline ((E.merger (subst P old c)).apply (subst P old c) S D).format
          ((E.merger (subst P old c)).apply (subst P old c) S D).text).length).filter (fun i =>
          ((E.merger (subst P old c)).origin (subst P old c) S D i).contains (decide (f = S), b.2))).map (D.path, ·)
      else [(((rewriter E (subst P old c)).apply (subst P old c) f).path, b.2)]
    | _, _ => [(((rewriter E (subst P old c)).apply (subst P old c) f).path, b.2)]

def preview (E : Engine) (G : Graph) (old : String) (c : Candidate) : List (BlockId × BlockId) :=
  (blocks P G).flatMap (fun b => ((images P E G old c b).filter (fun b' =>
    decide (signature P G b ≠ signature P (execute P E G old c) b'))).map (b, ·))

/-- Lost and newly activated blocks are separate preview rows. No absent
origin silently erases a reference from the dialog. -/
def lost (E : Engine) (G : Graph) (old : String) (c : Candidate) : List BlockId :=
  (blocks P G).filter (fun b => (images P E G old c b).isEmpty)
def introduced (E : Engine) (G : Graph) (old : String) (c : Candidate) : List BlockId :=
  (blocks P (execute P E G old c)).filter (fun b' =>
    !((blocks P G).flatMap (images P E G old c)).contains b')

end Consolidated13
end CM
