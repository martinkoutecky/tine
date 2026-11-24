import CutDecision
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)
def singleLine (s : String) : Bool := !s.toList.contains '\n' && !s.toList.contains '\r'
def edits (O : Observations) (old : String) (p : Plan) (f : File) : List Edit :=
  let c := O.content f
  let atoms := c.header.flatMap Header.text ++ c.blocks.flatMap Block.body
  (atoms.filterMap (fun a =>
    let result := mapAtom P D O f.format old p (skipFile P D O old p f) a
    match a with
    | .text _ => none
    | .site s | .title s _ =>
      if atomText result = s.native then none else some ⟨s.start,s.stop,atomText result⟩)).foldr insertEdit []

/-- Coordinates are unique between independent atoms and between sibling title
parts. The enclosing title may coincide with a nested reference. -/
def independentSites : List Atom → Bool
  | [] => true
  | a :: rest =>
    let nested : List (Nat × Nat) := match a with
      | .title _ ps => ps.filterMap (fun (p : String ⊕ Site) => match p with | .inl _ => none | .inr s => some (s.start,s.stop))
      | _ => []
    decide nested.Nodup &&
    (atomSites a).all (fun s => (rest.flatMap atomSites).all (fun t => (s.start,s.stop) != (t.start,t.stop))) &&
    independentSites rest

def validSites (O : Observations) (f : File) : Bool :=
  let sites := contentSites (O.content f)
  let c := O.content f
  (c.header.flatMap Header.text ++ c.blocks.flatMap Block.body).all titleCoherent &&
  independentSites (c.header.flatMap Header.text ++ c.blocks.flatMap Block.body) &&
  sites.all (fun s => decide (s.start < s.stop ∧ s.stop ≤ (bytes f.text).length) &&
    decide (((bytes f.text).drop s.start).take (s.stop - s.start) = bytes s.native))

def editGeometry (text : String) : Nat → List Edit → Bool
  | _, [] => true
  | cursor, e :: es => decide (cursor ≤ e.start ∧ e.start < e.stop ∧ e.stop ≤ (bytes text).length) &&
      ((bytes text).drop e.start |>.take (e.stop - e.start)).all (fun c => c != 10 && c != 13) &&
      singleLine e.replacement && editGeometry text e.stop es

def inPair (p : Plan) (f : File) : Bool := p.survivor.isSome && decide (some f = p.source ∨ some f = p.survivor)

def p1File (O : Observations) (old : String) (p : Plan) (f h : File) : Bool :=
  if inPair p f then true else
    let es := edits P D O old p f
    editGeometry f.text 0 es && decide (bytes h.text = splice (bytes f.text) 0 es)

def p1Check (O : Observations) (G H : Graph) (old : String) (p : Plan) : Bool :=
  ((retained G p).zip H).all (fun (f,h) => p1File P D O old p f h)

/-- Exact-splice statement: all gaps are original input slices, at monotonically
increasing coordinates; every replaced old slice and replacement is single-line. -/
theorem p1_splice {O : Observations} {old : String} {p : Plan} {f h : File}
    (hn : inPair p f = false) (hc : p1File P D O old p f h = true) :
    editGeometry f.text 0 (edits P D O old p f) = true ∧
    bytes h.text = splice (bytes f.text) 0 (edits P D O old p f) := by
  have h := hc
  simp only [p1File,hn,Bool.false_eq_true,if_false,Bool.and_eq_true,decide_eq_true_eq] at h
  exact h

theorem bytes_injective {s t : String} (h : bytes s = bytes t) : s = t := by
  exact String.toByteArray_inj.mp (congrArg ByteArray.mk (Array.toList_inj.mp h))

theorem p1_unrewritten {O : Observations} {old : String} {p : Plan} {f h : File}
    (hn : inPair p f = false) (he : edits P D O old p f = [])
    (hc : p1File P D O old p f h = true) : h.text = f.text := by
  have hh := (p1_splice P D hn hc).2
  simp only [he,splice,List.drop_zero] at hh
  exact bytes_injective hh

/-- Coordinates are not fabricated report labels: admission checks uniqueness
of the actual input coordinates after selecting the actual exception sites. -/
def reportsDistinct (rs : List Report) : Bool := decide ((rs.map (fun r => (r.path,r.start,r.stop,r.kind))).Nodup)
end Cut
end CM
