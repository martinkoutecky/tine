import ArbitraryInitialDisk

namespace ArbitraryInitialDiskWitness
open Storage21 (Config RealByte absent noneBytes unknown)
open Storage35

def config : Config 2 := { positive := by decide, texts := [0], nonempty := by decide }
def source : Fin 2 := 0
def destination : Fin 2 := 1
def disk (p : Fin 2) : Int := if p = source then 0 else absent

def move : Action 2 :=
  .opApply (fun p => some (if p = source then absent else 0))
    (fun p => some (disk p)) (some destination) (some source)

theorem admits_move :
    disk ≠ Storage31.initDisk config ∧
    config.mutant = "none" ∧ (∀ p, RealByte config (disk p)) ∧
    absent ∉ config.texts ∧ noneBytes ∉ config.texts ∧ unknown ∉ config.texts ∧
    ∃ loaded admitted : State 2,
      Reachable config disk loaded ∧ step config loaded move = some admitted ∧
      Reachable config disk admitted ∧
      admitted.g.renamed = [(source, destination)] ∧
      (admitted.s.drafts source).bytes = absent ∧ (admitted.s.drafts destination).bytes = 0 ∧
      guarantee admitted = true ∧ orderHolds admitted = true ∧ renameResolves admitted = true := by
  have different : disk ≠ Storage31.initDisk config := by
    intro eq
    exact (by decide : disk source ≠ Storage31.initDisk config source) (congrFun eq source)
  have realDisk : ∀ p, RealByte config (disk p) := by
    intro p
    by_cases eq : p = source <;> simp [disk, eq, RealByte, config]
  let initial := initWith config disk
  let loadSource : Action 2 := .prior (.load source)
  let loadDestination : Action 2 := .prior (.load destination)
  let withSource := execute config initial loadSource
  let loaded := execute config withSource loadDestination
  let admitted := execute config loaded move
  have sourceEnabled : enabled config initial loadSource = true := by decide
  have destinationEnabled : enabled config withSource loadDestination = true := by decide
  have moveEnabled : enabled config loaded move = true := by decide
  have sourceReach : Reachable config disk withSource := by
    apply Reachable.advance .initial loadSource
    change step config initial loadSource = some withSource
    simp [step, sourceEnabled, withSource]
  have loadedReach : Reachable config disk loaded :=
    .advance sourceReach loadDestination (by simp [step, destinationEnabled, loaded])
  have admission : step config loaded move = some admitted := by simp [step, moveEnabled, admitted]
  have admittedReach : Reachable config disk admitted := .advance loadedReach move admission
  refine ⟨different, rfl, realDisk, by decide, by decide, by decide, loaded, admitted,
    loadedReach, admission, admittedReach, by decide, by decide, by decide, ?_⟩
  exact all_targets_arbitrary_initial_disk config disk admitted rfl realDisk
    (by decide) (by decide) (by decide) admittedReach

#print axioms admits_move
end ArbitraryInitialDiskWitness
