import Guarantee31
import Lean
namespace Storage31
open Storage21 Storage3 Lean Elab Command
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000

def uniformConfig7 (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) : Config P :=
  {positive := pos, texts := positiveTexts4 T,
   nonempty := by simp [positiveTexts4]; omega,
   r1,weak,crashes,powers,mutant := "none"}

theorem uniform_guarantee7 (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) (x : State P)
    (hr : Storage31.Reachable (uniformConfig7 P T pos two r1 weak crashes powers)
      (initDisk (uniformConfig7 P T pos two r1 weak crashes powers)) x) :
    Storage3.noLoss x = true ∧ x.g.bad = [] ∧ Storage3.accepted x = true ∧ Storage3.trashed x = true := by
  have h := guarantee_positiveTexts7 (uniformConfig7 P T pos two r1 weak crashes powers) T rfl two rfl x hr
  simpa [Storage31.guarantee,Storage3.guarantee,Storage3.transitions,Storage21.transitions,
    project,Bool.and_eq_true,and_assoc] using h

theorem uniform_initial7 (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) :
    Storage31.Reachable (uniformConfig7 P T pos two r1 weak crashes powers)
      (initDisk (uniformConfig7 P T pos two r1 weak crashes powers))
      (Storage31.init (uniformConfig7 P T pos two r1 weak crashes powers)) := .initial

#print axioms uniform_guarantee7
#print axioms uniform_initial7
-- Traverse actual types and proof/definition values, without the imported axiom cache.
run_cmd do
  let env ← getEnv
  let mut pending := [``Storage31.guarantee_positiveTexts7]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  let mut localCount := 0
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "Missing checked constant {n}"
    if info.isUnsafe then throwError "Unsafe constant in proof cone: {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "Nonstandard axiom in raw cone: {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some value := info.value? (allowOpaque := true) then
      pending := value.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
    if n.toString.startsWith "Storage31." || n.toString.startsWith "Storage3." || n.toString.startsWith "Storage21." then
      localCount := localCount + 1
      logInfo m!"CONE7 {n}"
  logInfo m!"RAW_CONE7_TOTAL {seen.size}; LOCAL {localCount}; AXIOMS {axs.toList}"

end Storage31
