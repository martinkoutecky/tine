/- Inhabited offered alias merge, with complete raw text and two header origins. -/

import DateTwins14

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace AliasRealization14
open Params Consolidated13 Date13 Merge7 AliasWitness14 QueryRename6

def src := S .md "\n" ""
def dst := D .md "\n" ""
def result := O .md "\n" ""
def header := "alias:: X\nstatus:: done"
def sourceBody := "- source body"
def targetBody := "- target body"
def leftover := "status:: done"
def outline (_ : Format) (text : String) :=
  if text == src.text then [header,sourceBody]
  else if text == dst.text then [header,targetBody]
  else if text == result.text then [header,targetBody,leftover,sourceBody] else [text]
def P : Inputs String := { Witness13.P with
  title := fun _ _ => none
  aliasDecl := fun _ text => if text.startsWith "alias:: X\n" then ["X"] else []
  outline := outline
  occ := fun _ _ => [] }
def dates : Dates String := ⟨fun _ => none,fun _ => none⟩
def names : MemberNames P dates where
  identifying := P.name
  identifies := by
    intro f
    simp only [Date13.key,Date13.claim,Date13.target,dates]
    split <;> rfl

def policy : DatePolicy String := ⟨fun _ _ => false⟩
def SP : StoredParser Unit := ⟨fun _ _ => []⟩
def Q : QParams String Unit := ⟨fun _ _ => [0,0,0,0],fun _ _ => true,fun _ _ _ => false,fun _ _ _ => false⟩
def R : Rulings9.QRewriter := ⟨fun m _ f => {f with path := m f.path}⟩
def J : (String → String) → Merger := fun _ =>
  ⟨fun _ _ _ => result,fun _ _ _ i => match i with
    | 0 => [(false,0),(true,0)] | 1 => [(false,1)] | 2 => [(true,0)] | _ => [(true,1)]⟩
def E : Engine := ⟨R,J⟩
def G : Graph := [src,dst]
def c := Date13.literal P dates policy G "A" "B"
def m := Date13.subst P dates policy "A" c

def AP : AliasParser := ⟨fun fmt text => if text.startsWith "alias:: X\n" then [⟨fmt,["X"],"\n"⟩] else []⟩
def T := trace .md "\n" ""

theorem chosen : c.survivor = some dst ∧ Date13.source P dates policy G "A" = some src ∧
    Date13.execute P dates policy E G "A" c = [result] ∧
    c ∈ Date13.offers P dates policy names SP E G [] "A" "B" := by decide +kernel

theorem faithful : QueryRename6.FaithfulOn P.toParams SP (Date13.rewriter E m) m G := by
  constructor
  · constructor
    all_goals intro f hf
    all_goals simp only [G,List.mem_cons,List.not_mem_nil,or_false] at hf
    all_goals rcases hf with rfl | rfl
    all_goals first | decide +kernel | (intro _; decide +kernel)
  · intro f hf
    simp only [G,List.mem_cons,List.not_mem_nil,or_false] at hf
    rcases hf with rfl | rfl <;> decide +kernel

theorem merge_laws : Date13.MergeRealization P AP (Date13.rewriter E m).base (E.merger m) m src dst := by
  constructor
  all_goals first | decide +kernel | skip
  all_goals intro i hi
  all_goals first
    | (change i < 4 at hi)
    | (change i < 2 at hi)
  all_goals have hc : i = 0 ∨ i = 1 ∨ i = 2 ∨ i = 3 := by omega
  all_goals rcases hc with rfl | rfl | rfl | rfl
  all_goals first | decide +kernel | (simp [survLen,P,outline,J,E,src,dst,result,S,D,O,file,sourcePieces,targetPieces] at hi)

def realization : Date13.Realizes P dates policy SP Q AP E G "A" c T.input T.role where
  rewrites := faithful
  trace := T
  fixedInput := rfl
  fixedRole := rfl
  fixedScope := by rw [chosen.2.1,chosen.1]; rfl
  text := by
    rw [chosen.2.2.1]
    have h := certificate .md "\n" "" (Or.inl rfl)
    -- All editable members are X, and the chosen substitution fixes X.
    constructor
    · exact h.before
    · intro f hf
      have hf' : f = result := by simpa using hf
      subst f; decide +kernel
    · exact h.partition
    · intro x hp
      simp only [T,trace,List.mem_cons,List.not_mem_nil,or_false] at hp
      rcases hp with rfl | rfl | rfl
      · left; exact ⟨rfl,p 0 "alias:: ",by decide +kernel,.md,rfl,rfl⟩
      · right; exact ⟨rfl,p 1 "X" true,by decide +kernel,rfl,rfl⟩
      · left; exact ⟨rfl,p 0 "alias:: ",by decide +kernel,.md,rfl,rfl⟩
    · exact h.inScope
  merge := by
    intro S D hs hd
    have hs' : S = src := (Option.some.inj (chosen.2.1.symm.trans hs)).symm
    have hd' : D = dst := (Option.some.inj (chosen.1.symm.trans hd)).symm
    subst S; subst D
    exact ⟨merge_laws,by decide +kernel,by decide +kernel⟩

theorem inhabited_offered_alias_merge :
    c ∈ Date13.offers P dates policy names SP E G [] "A" "B" ∧
    Nonempty (Date13.Realizes P dates policy SP Q AP E G "A" c T.input T.role) :=
  ⟨chosen.2.2.2,⟨realization⟩⟩

end AliasRealization14
end CM
