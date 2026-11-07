/- Kernel-executed witnesses for every final comparison input family. -/
import DateComparison13

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace DateComparisonWitness13
open Params Consolidated13
open Witnesses2 (pg)
abbrev D := DateWitness13.D
abbrev Q := Witness11.Q
def base : Inputs String := Witness13.P
def P : Inputs String := { base with
  fileLe := claimantLe base
  refKind := fun k => if (D.canonical k).isSome then .journal else .page }

theorem no_normalization_empty : ¬ readerNormalization P [] := by
  have hp : Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P [])) P.pageLe = P.pageLe := by
    funext p q
    simp [Owner5.rankLe, Owner5.pathRank, Params.owner, minBy, Params.claimants, activeGraph]
  unfold readerNormalization
  intro h
  apply h
  unfold readParams
  rw [hp]
  rfl

/-- The verifier's counterexample, now covered by the target diagnostic alone. -/
theorem target_only_navigation :
    activeGraph P [] = [] ∧ emptyReal P [] "2026_11_06" = false ∧
    Date13.navigationCases P D [] "2026_11_06" =
      [("navigation target", true), ("navigation direct declarer", false)] ∧
    P.resolve [] "2026_11_06" = (.journal, "2026_11_06") ∧
    Date13.resolve P D [] "2026_11_06" = (.journal, "Nov 6th, 2026") := by decide +kernel

def declaring : Graph := [pg "A" "a:2026_11_06"]
def DP : Inputs String := { P with
  pageLe := Owner5.rankLe (Owner5.pathRank (physicalParams P) declaring) P.pageLe }

theorem rank_idempotent {α : Type} (r : α → List Nat) (le : α → α → Bool) :
    Owner5.rankLe r (Owner5.rankLe r le) = Owner5.rankLe r le := by
  funext a b
  by_cases h : r a = r b <;> simp [Owner5.rankLe, h]

theorem no_normalization_declaring : ¬ readerNormalization DP declaring := by
  have ha : activeGraph DP declaring = declaring := by decide +kernel
  have hp : Owner5.rankLe (Owner5.pathRank (physicalParams DP) (activeGraph DP declaring)) DP.pageLe = DP.pageLe := by
    rw [ha]
    change Owner5.rankLe (Owner5.pathRank (physicalParams P) declaring)
      (Owner5.rankLe (Owner5.pathRank (physicalParams P) declaring) P.pageLe) = _
    exact rank_idempotent ..
  intro h
  apply h
  unfold readParams
  rw [hp]
  rfl

/-- The target is identical; only interpretation of the direct declaration
changes. None of the three old static diagnostics holds. -/
theorem direct_only_navigation :
    activeGraph DP declaring = declaring ∧ emptyReal DP declaring "Nov 6th, 2026" = false ∧
    Date13.target DP D declaring "Nov 6th, 2026" = Date13.genericTarget DP declaring "Nov 6th, 2026" ∧
    Date13.navigationCases DP D declaring "Nov 6th, 2026" =
      [("navigation target", false), ("navigation direct declarer", true)] ∧
    DP.resolve declaring "Nov 6th, 2026" = (.journal, "Nov 6th, 2026") ∧
    Date13.resolve DP D declaring "Nov 6th, 2026" = (.page, "A") := by decide +kernel

/-- Diagnostics are conservative causes: a shared direct declarer can mask
a changed target, so a positive target flag does not assert unequal outputs. -/
theorem masked_target :
    Date13.navigationCases DP D declaring "2026_11_06" =
      [("navigation target", true), ("navigation direct declarer", false)] ∧
    Date13.resolve DP D declaring "2026_11_06" = DP.resolve declaring "2026_11_06" := by decide +kernel

def seedGraph : Graph := [pg "R" "l:2026_11_06"]
theorem seed_witness :
    "2026_11_06" ∈ Date13.rawSeeds P seedGraph ∧
    Date13.finalSeed P D seedGraph "2026_11_06" = some (.journal, "Nov 6th, 2026") ∧
    Date13.genericSeed P seedGraph "2026_11_06" = some (.journal, "2026_11_06") := by decide +kernel

theorem reference_path_witness :
    "2026_11_06" ∈ P.refNames (activeGraph P seedGraph) ("R", 0) ∧
    sameCls (Date13.edges P D seedGraph) (Date13.target P D seedGraph "2026_11_06")
      (.journal, "Nov 6th, 2026") = true ∧
    sameCls ((readParams P seedGraph).aliasEdges (activeGraph P seedGraph)) (P.key "2026_11_06")
      "Nov 6th, 2026" = false := by decide +kernel

theorem query_leaf_witness :
    Node.page (.journal, "Nov 6th, 2026") ∈ Date13.univ DateWitness13.P D DateWitness13.canonicalOnly .page ∧
    Date13.leafAt DateWitness13.P D Q DateWitness13.canonicalOnly (.name "Nov 6th, 2026")
      (.page (.journal, "Nov 6th, 2026")) = false ∧
    (readParams DateWitness13.P DateWitness13.canonicalOnly).leafAt3 Q
      (activeGraph DateWitness13.P DateWitness13.canonicalOnly) (.name "Nov 6th, 2026")
      (.page (.journal, "Nov 6th, 2026")) = true := by decide +kernel

theorem panel_source_witness :
    ("Nov 6th, 2026", 0) ∈ Consolidated13.blocks DateWitness13.P DateWitness13.canonicalOnly ∧
    Date13.panelSource DateWitness13.P D none DateWitness13.canonicalOnly
      (.journal, "Nov 6th, 2026") ("Nov 6th, 2026", 0) = true ∧
    Consolidated13.panelSource DateWitness13.P none DateWitness13.canonicalOnly
      (.journal, "Nov 6th, 2026") ("Nov 6th, 2026", 0) = false := by decide +kernel

def duplicatePaths : Graph := [pg "shared" "t:A|l:T", pg "shared" "t:B|l:T"]
theorem block_source_witness :
    (duplicatePaths.map File.path).Nodup = False ∧
    Date13.finalBlockSource P duplicatePaths (.page, "B") ("shared", 0) = false ∧
    Date13.genericBlockSource P duplicatePaths (.page, "B") ("shared", 0) = true ∧
    Date13.links P D {favoritesPage := ""} duplicatePaths (.page, "B") (.page, "T") = false ∧
    Consolidated13.links P {favoritesPage := ""} duplicatePaths (.page, "B") (.page, "T") = true := by decide +kernel

theorem parent_lookup_witness :
    Date13.finalParentLookup DateWitness13.P D DateWitness13.canonicalOnly "2026_11_06" = none ∧
    Date13.genericParentLookup DateWitness13.P DateWitness13.canonicalOnly "2026_11_06" =
      some (.page, "2026_11_06") := by decide +kernel

def structural : Graph := [pg "S" "l:2026_11_06", pg "2026_11_06/x" "body"]
theorem links_tree_near_differences :
    Date13.links P D {favoritesPage := ""} structural (.page, "S") (.journal, "Nov 6th, 2026") = true ∧
    Decided.linkB P.toParams {favoritesPage := ""} structural (.page, "S") (.journal, "Nov 6th, 2026") = false ∧
    Date13.treeParent P D id structural (.page, "2026_11_06/x") = some (.journal, "Nov 6th, 2026") ∧
    Rulings11.treeParent P.toParams id structural (.page, "2026_11_06/x") = some (.journal, "2026_11_06") ∧
    Date13.tree P D id structural ≠ Rulings11.tree P.toParams id structural ∧
    Date13.namespaceTree P D id structural ≠ Rulings11.namespaceTree P.toParams id structural ∧
    Date13.near P D {favoritesPage := ""} structural .out 1 [(.page, "S")] ≠
      Decided.nearL P.toParams {favoritesPage := ""} structural .out 1 [(.page, "S")] := by decide +kernel

theorem structural_diagnostic : Date13.readerDifference P D Q none structural := by
  left
  refine ⟨"2026_11_06", ?_, ?_⟩ <;> decide +kernel

/-- The equality branch is inhabited. Query diagnostics inspect actual anchor
nodes; a differing leaf on a node outside the universe is not a cause. -/
theorem no_readerDifference_empty : ¬ Date13.readerDifference P D Q none [] := by
  simp [Date13.readerDifference, Date13.rawSeeds, Date13.univ, Date13.pages,
    Date13.seedTargets, Date13.panelSource, Consolidated13.panelSource,
    Date13.finalBlockSource, Date13.genericBlockSource,
    Date13.finalParentLookup, Date13.genericParentLookup, Consolidated13.pages,
    Decided.pages, Params.pagesR, Params.realPages, Params.refOnlyPagesR,
    Params.pageSeedKeys, Params.refKeys, Params.nameKeys, Params.pageBlocks,
    Params.owner, Params.claimants, Params.blocks, Consolidated13.blocks,
    Params.refNames, Params.occAt, fileAt, activeGraph, minBy]
  intro a n
  cases a <;> simp

theorem empty_reader_agreement :
    Date13.StaticAgreement P D Q {favoritesPage := ""} none id [] "Unknown" := by
  rcases Date13.static_comparison_exhaustive_final P D Q {favoritesPage := ""} none id [] "Unknown" with
    ha | hn | he | hd | hv | h
  · exact False.elim (ha rfl)
  · exact False.elim (no_normalization_empty hn)
  · have hf : emptyReal P [] "Unknown" = false := by decide +kernel
    rw [hf] at he; contradiction
  · exact False.elim (no_readerDifference_empty hd)
  · have hf : ¬ anyDifference (Date13.navigationCases P D [] "Unknown") := by
      rw [Date13.navigationCases_exact]
      decide +kernel
    exact False.elim (hf hv)
  · exact h

end DateComparisonWitness13
end CM
