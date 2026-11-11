/- Concrete full-text certificates, in both native formats. -/
import Alias14

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace AliasWitness14
open Consolidated13 Alias14

def p (id : Nat) (text : String) (edit : Bool := false) : Piece := ⟨id,text,edit⟩
def targetPieces (fmt : Format) (ending extra : String) :=
  [p 0 (fieldPrefix fmt),p 1 "X" true,p 2 ending,p 3 ("status:: done\n- target body\n" ++ extra)]
def sourcePieces (fmt : Format) (ending extra : String) :=
  [p 10 (fieldPrefix fmt),p 11 "X" true,p 12 ending,p 13 ("status:: done\n- source body\n" ++ extra)]
def file (path : String) (fmt : Format) (pieces : List Piece) : File :=
  ⟨path,.pages,fmt,String.join (pieces.map Piece.text)⟩
def S (fmt : Format) (ending extra : String) := file "A" fmt (sourcePieces fmt ending extra)
def D (fmt : Format) (ending extra : String) := file "B" fmt (targetPieces fmt ending extra)
def outPieces (fmt : Format) (ending extra : String) : List Placement :=
  (targetPieces fmt ending extra).map Placement.original ++ [.original (p 13 ("status:: done\n- source body\n" ++ extra))]
def O (fmt : Format) (ending extra : String) : File :=
  ⟨"B",.pages,fmt,String.join ((outPieces fmt ending extra).map (Placement.render id))⟩
def role (fmt : Format) (x : Piece) : Role :=
  if x.id == 0 || x.id == 10 then .key fmt
  else if x.id == 1 || x.id == 11 then .name
  else if x.id == 2 || x.id == 12 then .delimiter else .opaqueText

def trace (fmt : Format) (ending extra : String) : Trace where
  input := fun f => if f.path == "A" then sourcePieces fmt ending extra else targetPieces fmt ending extra
  role := role fmt
  output := fun _ => outPieces fmt ending extra
  consumed := [p 10 (fieldPrefix fmt),p 11 "X" true,p 12 ending]
  scope := [S fmt ending extra,D fmt ending extra]

theorem reorder (a b c d e f g h : Piece) :
    [a,b,c,d,e,f,g,h].Perm [e,f,g,h,d,a,b,c] := by
  have hp : ([0,1,2,3,4,5,6,7] : List Nat).Perm [4,5,6,7,3,0,1,2] := by decide +kernel
  exact hp.map (fun i => match i with | 0 => a | 1 => b | 2 => c | 3 => d | 4 => e | 5 => f | 6 => g | _ => h)

theorem certificate (fmt : Format) (ending extra : String)
    (he : ending = "\n" ∨ ending = "\r\n") :
    Valid (trace fmt ending extra) id [S fmt ending extra,D fmt ending extra] [O fmt ending extra] := by
  constructor
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    rcases hf with rfl | rfl <;> rfl
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    subst f; rfl
  · simpa [trace,retained,outPieces,S,D,O,file,targetPieces,sourcePieces,Placement.original?] using
      reorder (p 10 (fieldPrefix fmt)) (p 11 "X" true) (p 12 ending)
        (p 13 ("status:: done\n- source body\n" ++ extra))
        (p 0 (fieldPrefix fmt)) (p 1 "X" true) (p 2 ending)
        (p 3 ("status:: done\n- target body\n" ++ extra))
  · intro x hx
    simp only [trace,List.mem_cons,List.not_mem_nil,or_false] at hx
    rcases hx with rfl | rfl | rfl
    · left
      refine ⟨?_,p 0 (fieldPrefix fmt),?_,fmt,?_,?_⟩
      all_goals cases fmt <;> simp [trace,role,p,isSyntax,fieldPrefix,retained,outPieces,targetPieces,Placement.original?]
    · right
      refine ⟨by simp [trace,role,p],p 1 "X" true,?_,by simp [trace,role,p],rfl⟩
      simp [trace,retained,outPieces,targetPieces,Placement.original?]
    · left
      refine ⟨?_,p 0 (fieldPrefix fmt),?_,fmt,?_,?_⟩
      all_goals cases fmt <;> rcases he with rfl | rfl <;>
        simp [trace,role,p,isSyntax,fieldPrefix,retained,outPieces,targetPieces,Placement.original?]
  · intro x hx
    refine ⟨S fmt ending extra,by simp [trace],by simp,?_⟩
    simp only [trace,List.mem_cons,List.not_mem_nil,or_false] at hx
    change x ∈ sourcePieces fmt ending extra
    simp only [sourcePieces,List.mem_cons,List.not_mem_nil,or_false]
    rcases hx with h | h | h
    · exact Or.inl h
    · exact Or.inr (Or.inl h)
    · exact Or.inr (Or.inr (Or.inl h))

theorem markdown_text :
    (S .md "\n" "").text = "alias:: X\nstatus:: done\n- source body\n" ∧
    (O .md "\n" "").text = "alias:: X\nstatus:: done\n- target body\nstatus:: done\n- source body\n" := by decide +kernel

theorem org_text :
    (S .org "\n" "").text = "#+alias: X\nstatus:: done\n- source body\n" ∧
    (O .org "\n" "").text = "#+alias: X\nstatus:: done\n- target body\nstatus:: done\n- source body\n" := by decide +kernel

/-- Deliberately awkward raw text is protected, including duplicate properties,
wrappers, spacing, quotes, examples, malformed remainders and missing final LF. -/
def awkward := "  [[X]]\nquoted:: \"not an alias\"\n```\nalias:: example\n```\nunknown [[\nno final newline"
theorem awkward_preserved :
    Valid (trace .md "\r\n" awkward) id [S .md "\r\n" awkward,D .md "\r\n" awkward] [O .md "\r\n" awkward] :=
  certificate .md "\r\n" awkward (Or.inr rfl)

theorem overlap_case_and_substitution :
    members id ["X","x"] ["X","Y"] = ["X","x","Y"] ∧
    members (fun s => if s == "A" then "B" else s) ["B"] ["A"] = ["B"] ∧
    members id ["[[X]]"] ["X"] = ["[[X]]","X"] ∧
    isSyntax .opaqueText "status:: done" = false ∧
    isSyntax .delimiter "  " = false ∧ isSyntax .delimiter "[[" = false := by decide +kernel

end AliasWitness14
end CM

namespace CM
namespace AliasExtra14
open Consolidated13 Alias14 AliasWitness14

def overlapSource : List Piece := [p 10 "alias:: ",p 11 "X" true,p 12 ", ",p 13 "Y" true,p 15 "\n",p 16 "status:: done\n- source\n"]
def overlapTarget : List Piece := [p 0 "alias:: ",p 1 "X" true,p 2 ", ",p 3 "x" true,p 4 "\n",p 5 "status:: done\n- target\n"]
def overlapOut : List Placement := [.original (p 0 "alias:: "),.original (p 1 "X" true),.original (p 2 ", "),.original (p 3 "x" true),.insert ", ",.original (p 13 "Y" true),.original (p 4 "\n"),.original (p 5 "status:: done\n- target\n"),.original (p 16 "status:: done\n- source\n")]
def overlapMap : String → String := id
def overlapS := file "A" .md overlapSource
def overlapD := file "B" .md overlapTarget
def overlapO : File := ⟨"B",.pages,.md,String.join (overlapOut.map (Placement.render overlapMap))⟩
def overlapT : Trace where
  input := fun f => if f.path == "A" then overlapSource else overlapTarget
  output := fun _ => overlapOut
  consumed := [p 10 "alias:: ",p 11 "X" true,p 12 ", ",p 15 "\n"]
  role := fun x => if x.id == 0 || x.id == 10 then .key .md
    else if [1,3,11,13].contains x.id then .name
    else if [2,4,12,15].contains x.id then .delimiter else .opaqueText
  scope := [overlapS,overlapD]

theorem overlap_certificate : Valid overlapT overlapMap [overlapS,overlapD] [overlapO] := by
  constructor
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    rcases hf with rfl | rfl <;> rfl
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    subst f; rfl
  · decide +kernel
  · intro x hx
    simp only [overlapT,List.mem_cons,List.not_mem_nil,or_false] at hx
    rcases hx with rfl | rfl | rfl | rfl
    all_goals first
      | (right; exact ⟨rfl,p 1 "X" true,by decide +kernel,rfl,by decide +kernel⟩)
      | (left; exact ⟨rfl,p 0 "alias:: ",by decide +kernel,.md,rfl,rfl⟩)
  · intro x hx
    simp only [overlapT,List.mem_cons,List.not_mem_nil,or_false] at hx
    rcases hx with rfl | rfl | rfl | rfl
    all_goals exact ⟨overlapS,by simp [overlapT],by simp,by decide +kernel⟩

def substitutionSource : List Piece := [p 10 "alias:: ",p 11 "A" true,p 15 "\n",p 16 "- source"]
def substitutionTarget : List Piece := [p 0 "alias:: ",p 1 "X" true,p 4 "\n",p 5 "- target\n"]
def substitutionOut : List Placement := [.original (p 0 "alias:: "),.original (p 1 "X" true),.original (p 4 "\n"),.original (p 5 "- target\n"),.original (p 16 "- source")]
def substitutionMap : String → String := (fun s => if s == "A" then "X" else s)
def substitutionS := file "A" .md substitutionSource
def substitutionD := file "B" .md substitutionTarget
def substitutionO : File := ⟨"B",.pages,.md,String.join (substitutionOut.map (Placement.render substitutionMap))⟩
def substitutionT : Trace where
  input := fun f => if f.path == "A" then substitutionSource else substitutionTarget
  output := fun _ => substitutionOut
  consumed := [p 10 "alias:: ",p 11 "A" true,p 15 "\n"]
  role := fun x => if x.id == 0 || x.id == 10 then .key .md
    else if [1,3,11,13].contains x.id then .name
    else if [2,4,12,15].contains x.id then .delimiter else .opaqueText
  scope := [substitutionS,substitutionD]

theorem substitution_certificate : Valid substitutionT substitutionMap [substitutionS,substitutionD] [substitutionO] := by
  constructor
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    rcases hf with rfl | rfl <;> rfl
  · intro f hf
    simp only [List.mem_cons,List.not_mem_nil,or_false] at hf
    subst f; rfl
  · decide +kernel
  · intro x hx
    simp only [substitutionT,List.mem_cons,List.not_mem_nil,or_false] at hx
    rcases hx with rfl | rfl | rfl
    all_goals first
      | (right; exact ⟨rfl,p 1 "X" true,by decide +kernel,rfl,by decide +kernel⟩)
      | (left; exact ⟨rfl,p 0 "alias:: ",by decide +kernel,.md,rfl,rfl⟩)
  · intro x hx
    simp only [substitutionT,List.mem_cons,List.not_mem_nil,or_false] at hx
    rcases hx with rfl | rfl | rfl
    all_goals exact ⟨substitutionS,by simp [substitutionT],by simp,by decide +kernel⟩

end AliasExtra14
end CM
