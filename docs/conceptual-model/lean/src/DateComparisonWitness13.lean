/- Primitive diagnostic witnesses, including navigation-only and masked changes. -/
import DateComparison13
import DateWitness13

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace DateComparisonWitness13
open Date13 DateWitness13

def rows : Consolidated13.QueryRowParser := ⟨fun _ _ _ => .body⟩
def emptyInputs := readerInputs P D Q rows {favoritesPage := ""} none id []

def oldEmpty := {emptyInputs with initial := fun s =>
  if s == "2026_11_02" then (.journal,"2026_11_02") else emptyInputs.initial s}

theorem target_only :
    oldEmpty.direct "2026_11_02" = emptyInputs.direct "2026_11_02" ∧
    oldEmpty.initial "2026_11_02" ≠ emptyInputs.initial "2026_11_02" ∧
    oldEmpty.opening "2026_11_02" ≠ emptyInputs.opening "2026_11_02" := by decide +kernel

def directOnly := {emptyInputs with direct := fun _ => some (.page,"Owner")}
theorem direct_only :
    directOnly.initial "X" = emptyInputs.initial "X" ∧
    directOnly.direct "X" ≠ emptyInputs.direct "X" ∧
    directOnly.opening "X" ≠ emptyInputs.opening "X" := by decide +kernel

def masked1 := {oldEmpty with direct := fun _ => some (.page,"Owner")}
def masked2 := {emptyInputs with direct := fun _ => some (.page,"Owner")}
theorem masked_target :
    masked1.initial "2026_11_02" ≠ masked2.initial "2026_11_02" ∧
    masked1.opening "2026_11_02" = masked2.opening "2026_11_02" := by decide +kernel

theorem normalization_diagnostic : primitiveDifference emptyInputs oldEmpty := by
  right; right; right; right; right; right; right; right; right; right; right; right; right; right
  left
  exact ⟨"2026_11_02",by decide +kernel⟩

/-- Every complete-reader surface agrees with itself; the certificate contains
all directions/radii/queries, not merely the selected fixture's query result. -/
theorem self_complete : ReaderAgreement emptyInputs emptyInputs :=
  ⟨rfl,rfl,rfl,rfl,rfl,fun _ => rfl,rfl,rfl,rfl,rfl,rfl,rfl⟩

end DateComparisonWitness13
end CM
