/- Rename/merge over normalized identities. The unresolved M2 policy is an
explicit argument; there is no selected date-source policy in this development. -/
import Date13

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6 Merge7
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- OPEN M2. A fixes every day; B days with real files; C journal-directory days.
No instance is selected. Physical journals remain fixed in every alternative. -/
structure DatePolicy (K : Type) where
  fixed : Graph → K → Bool

def isFixed (M : DatePolicy K) (G : Graph) (t : Page K) : Bool :=
  t.1 == .journal && M.fixed G t.2

def key (D : Dates K) (s : String) : Page K := target P D [] s

def movedNames (D : Dates K) (G : Graph) (old : String) : List String :=
  (Decided.usedNames P.toParams (activeGraph P D G)).filter (fun s =>
    (nsMatch (key P D) P.urlHasOrigin old s).isSome &&
    (pages P D G).contains (key P D s))

def subst (dates : Dates K) (M : DatePolicy K) (old : String) (c : Candidate) :=
  nsSubst (key P dates) P.urlHasOrigin old c.destination

def rewriter (E : Engine) (m : String → String) := E.rewrite.at m

def uniqueClaim (dates : Dates K) (M : DatePolicy K) (G : Graph) (k : Page K) : Option File :=
  match G.filter (fun f => decide (claim P dates f = k)) with
  | [f] => some f
  | _ => none

def source (dates : Dates K) (M : DatePolicy K) (G : Graph) (old : String) : Option File :=
  (uniqueClaim P dates M G (key P dates old)).filter (fun f => kindOf f == .page && !isFixed M G (claim P dates f))

/-- Literal names never route to an alias owner. A stationary literal file
receives source content, if there is source content. A moving own child does not. -/
def literal (dates : Dates K) (M : DatePolicy K) (G : Graph) (old new : String) : Candidate :=
  ⟨new, if (source P dates M G old).isSome && decide (key P dates old ≠ key P dates new) then
    (uniqueClaim P dates M G (key P dates new)).filter (fun f =>
      decide (f.dir = .journals) || decide (key P dates (nsSubst (key P dates) P.urlHasOrigin old new (P.name f)) = claim P dates f))
    else none⟩

/-- An accepted identifying spelling for each physical member, independently
of its displayed label. This is an operation adapter boundary, not M2 or a
parse/render law for labels. The reader does not require this adapter. -/
structure MemberNames (dates : Dates K) where
  identifying : File → String
  identifies : ∀ f, key P dates (identifying f) = claim P dates f

/-- Keep the authored effective spelling when identifying; otherwise use the
certified spelling. The chosen spelling drives the ENTIRE substitution. -/
def memberSpelling (dates : Dates K) (N : MemberNames P dates) (f : File) : String :=
  if key P dates (P.name f) = claim P dates f then P.name f else N.identifying f

/-- Literal destination plus EVERY distinct real member of its component.
The member's identity is never reconstructed from a non-identifying label. -/
def candidates (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates)
    (G : Graph) (old new : String) : List Candidate :=
  (literal P dates M G old new :: ((groupFiles P dates G new).filter (fun f =>
    decide (claim P dates f ≠ key P dates old))).map
      (fun f => literal P dates M G old (memberSpelling P dates N f))).eraseDups

def execute (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : Graph :=
  match source P dates M G old, c.survivor with
  | some S, some D => mergeBy (rewriter E (subst P dates M old c)).base
      (E.merger (subst P dates M old c)) (subst P dates M old c) S D G
  | _, _ => renameBy (rewriter E (subst P dates M old c)).base (subst P dates M old c) G

def directNames (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (G : Graph) : List String :=
  G.flatMap (fun f => P.name f :: P.aliasDecl f.format f.text ++
    (P.outline f.format f.text).flatMap (fun b =>
      (P.occ f.format b).map Prod.snd ++
      (SP.extract f.format b).flatMap queryArgs))

def anchors (dates : Dates K) (M : DatePolicy K) (G : Graph) (names : List String) : List (Page K) :=
  (G.filter (fun f => f.dir == .journals)).map (claim P dates) ++
  ((names.map (key P dates) ++ G.map (claim P dates)).filter (isFixed M G)) ++
  (G.flatMap (fun f => (P.outline f.format f.text).flatMap (fun b =>
    ((P.occ f.format b).filter (fun o => !Decided.covers o.1)).map (fun o => key P dates o.2))))

def demands (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (G : Graph) (extras : List String) (old : String) (c : Candidate) : List (Page K × Page K) :=
  (key P dates old, key P dates c.destination) ::
  ((directNames P dates M SP G ++ extras).map (fun s => (key P dates s, key P dates (subst P dates M old c s)))) ++
  (anchors P dates M G (old :: directNames P dates M SP G ++ extras)).map (fun t => (t,t))

def supported (dates : Dates K) (_M : DatePolicy K) (G : Graph) (old : String) : Bool :=
  (pages P dates G ++ (edges P dates G).flatMap (fun e => [e.1,e.2])).contains (key P dates old)

/-- Original and image conflicts: includes alias-only images (the merge
conflict omission is deliberately repaired). Unrelated conflicts remain local. -/
def touched (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (G : Graph) (extras : List String) (old : String) (c : Candidate) : List (Page K) :=
  let names := old :: c.destination ::
    ((Decided.usedNames P.toParams G ++ directNames P dates M SP G ++ extras).filter (fun s =>
      (nsMatch (key P dates) P.urlHasOrigin old s).isSome))
  names.flatMap (fun s => [key P dates s, key P dates (subst P dates M old c s)])

def localClean (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (G : Graph) (extras : List String) (old : String) (c : Candidate) : Bool :=
  (touched P dates M SP G extras old c).all (fun k => !conflicted P dates G k)

def pairReady (dates : Dates K) (M : DatePolicy K) (G : Graph) (old : String) (c : Candidate) : Bool :=
  match c.survivor with
  | none => true
  | some D => match source P dates M G old with
    | none => false
    | some S => decide (S ≠ D ∧ S.format = D.format) &&
      decide (uniqueClaim P dates M G (key P dates c.destination) = some D) &&
      (decide (D.dir = .journals) || decide (key P dates (subst P dates M old c (P.name D)) = claim P dates D))

def kept (dates : Dates K) (M : DatePolicy K) (G : Graph) (old : String) (c : Candidate) : Graph :=
  if c.survivor.isSome then G.filter (fun f => decide (source P dates M G old ≠ some f)) else G

def outputFile (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) (f : File) : File :=
  match source P dates M G old, c.survivor with
  | some S, some D => if f = D then (E.merger (subst P dates M old c)).apply (subst P dates M old c) S D
      else (rewriter E (subst P dates M old c)).apply (subst P dates M old c) f
  | _, _ => (rewriter E (subst P dates M old c)).apply (subst P dates M old c) f

def ownership (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : Bool :=
  (kept P dates M G old c).all (fun f => (kept P dates M G old c).all (fun g =>
    decide (claim P dates (outputFile P dates M E G old c f) =
      claim P dates (outputFile P dates M E G old c g) → claim P dates f = claim P dates g)))

/-- Virtual page descendants may not acquire stationary content. Root joining
is explicit in the candidate; alias-only names are not pages. -/
def noCapture (dates : Dates K) (M : DatePolicy K) (G : Graph) (old : String) (c : Candidate) : Bool :=
  (movedNames P dates G old).all (fun s =>
    G.all (fun f =>
      decide (key P dates s = key P dates old) ||
      decide (key P dates (subst P dates M old c s) = key P dates s) ||
      (decide (kindOf f = .page) &&
        decide (key P dates (subst P dates M old c (P.name f)) ≠ claim P dates f)) ||
      decide (claim P dates f ≠ key P dates (subst P dates M old c s))))

def safe (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (E : Engine) (G : Graph) (extras : List String) (old : String) (c : Candidate) : Bool :=
  supported P dates M G old && consistent (demands P dates M SP G extras old c) &&
    localClean P dates M SP G extras old c && pairReady P dates M G old c &&
    ownership P dates M E G old c && noCapture P dates M G old c &&
    decide ((G.map File.path).Nodup) && decide (((execute P dates M E G old c).map File.path).Nodup)

def offers (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L) (E : Engine) (G : Graph) (extras : List String) (old new : String) : List Candidate :=
  (candidates P dates M N G old new).filter (safe P dates M SP E G extras old)

/-- A preference is not an authorization; an unsafe default is absent. -/
def defaultCandidate (dates : Dates K) (M : DatePolicy K) (SP : StoredParser L) (E : Engine) (G : Graph) (extras : List String) (old new : String) : Option Candidate :=
  let c := literal P dates M G old new
  if safe P dates M SP E G extras old c then some c else none

def answer (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L) (E : Engine) (G : Graph) (extras : List String) (old new : String)
    (choice : Option Candidate) : Option Graph :=
  match choice with
  | none => some G
  | some c => if c ∈ offers P dates M N SP E G extras old new then some (execute P dates M E G old c) else none

def automatic (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L) (E : Engine) (G : Graph) (extras : List String) (old new : String) : Option Graph :=
  match offers P dates M N SP E G extras old new with
  | [c] => some (execute P dates M E G old c)
  | _ => none


end Date13
end CM
