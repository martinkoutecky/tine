import PairSync11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def CoreOnly11 : Storage21.Action P → Prop
  | .deliverUp _ | .flush _ | .rename | .dirSync _ | .crash | .switchFin | .power _ | .launch | .extWriteD _ _ _ => False
  | _ => True

theorem pairSafety_oldCore11 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (frame : CoreOnly11 a) :
    PairSafety10 (coreLift11 x (Storage21.execute c (core11 x) a)) := by
  cases a <;> simp only [Storage21.execute]
  case deliverUp ok => exact False.elim frame
  case flush p => exact False.elim frame
  case rename => exact False.elim frame
  case dirSync ok => exact False.elim frame
  case crash => exact False.elim frame
  case switchFin => exact False.elim frame
  case power keep => exact False.elim frame
  case launch => exact False.elim frame
  case extWriteD p b dur => exact False.elim frame
  case observe p =>
    apply pairSafety_core11 x _ h rfl
    · apply pageSafe_put11
      · exact fun q => pageSafe_refl11 _ _
      · exact pageSafe_table11 c _ _ _ mutant
    · exact fun on => ⟨on,rfl,rfl⟩
  case check =>
    simp only [Storage21.check]
    split
    · apply pairSafety_core11 x _ h rfl
      · apply pageSafe_put11
        · exact fun q => pageSafe_refl11 _ _
        · exact pageSafe_table11 c _ _ _ mutant
      · intro on; cases on
    · apply pairSafety_core11 x _ h rfl
      · exact fun q => pageSafe_refl11 _ _
      · exact fun on => ⟨on,rfl,rfl⟩
  case saveFail =>
    apply pairSafety_core11 x _ h rfl
    · apply pageSafe_put11
      · exact fun q => pageSafe_refl11 _ _
      · exact pageSafe_reply11 c _ _ _ "Failed" mutant (by decide)
    · intro on; cases on
  case switchReq =>
    apply pairSafety_core11 x _ h rfl
    · intro q
      change PageSafe11 _ _ (if _ then _ else _)
      split
      · exact pageSafe_risk11 _ _
      · exact pageSafe_refl11 _ _
    · exact fun on => ⟨on,rfl,rfl⟩
  all_goals apply pairSafety_core11 x _ h rfl
  all_goals first | exact fun q => pageSafe_refl11 _ _ | exact fun on => ⟨on,rfl,rfl⟩

theorem pairSafety_oldFrame11 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (frame : CoreOnly11 a) :
    PairSafety10 (execute c x (.prior (.old a))) := by
  apply (pairSafety_view11 _ (coreLift11 x (Storage21.execute c (core11 x) a)) ?_).mpr
    (pairSafety_oldCore11 c x a mutant h frame)
  cases a <;> try rfl
  all_goals try exact ⟨rfl,rfl,rfl,rfl,rfl,rfl,rfl⟩
  all_goals first | exact False.elim frame | skip
  case saveFail =>
    simp [PairViewEq11,execute,Storage35Base.execute,Storage32.execute,Storage32.saveFail,
      Storage32.lift,mutant,coreLift11,core11,Storage34.lift,Storage3.executeOld,Storage3.lift,
      Storage21.execute,Storage21.saveFail,Storage21.commit,project,Storage34.project,Storage32.project,Storage3.project]
  case check =>
    simp [PairViewEq11,execute,Storage35Base.execute,Storage32.execute,Storage32.check,
      Storage32.lift,mutant,coreLift11,core11,Storage34.lift,Storage3.executeOld,Storage3.lift,
      Storage21.execute,project,Storage34.project,Storage32.project,Storage3.project]
end Storage35
