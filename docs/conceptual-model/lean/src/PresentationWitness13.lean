/- Discriminating, kernel-evaluated presentation instances in the generic historical library. -/
import Twins13

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace PresentationWitness13
open Params Consolidated13
open Witnesses2 (pg)
abbrev P := Witness13.P
abbrev E := Witness13.E
abbrev SP := Witness13.SP

def panelGraph : Graph := [pg "A" "a:B|a:X|l:A|l:A;l:A", pg "B" "l:A",
  pg "Favorites" "a:F|l:A", pg "R" "l:A"]

theorem shared_relation_different_panels :
    relationHits P panelGraph (.page, "A") = [("A", 0), ("A", 1), ("B", 0), ("Favorites", 0), ("R", 0)] ∧
    relationHits P panelGraph (.page, "A") = relationHits P panelGraph (.page, "B") ∧
    linked P (some "Favorites") panelGraph (.page, "A") = [("B", 0), ("R", 0)] ∧
    linked P (some "Favorites") panelGraph (.page, "B") = [("A", 0), ("A", 1), ("R", 0)] ∧
    referenceCount P (some "Favorites") panelGraph (.page, "A") = 2 ∧
    referenceCount P (some "Favorites") panelGraph (.page, "B") = 3 := by decide +kernel

theorem alias_only_panel_self :
    panelSelfKey P panelGraph (.page, "X") = "A" ∧
    linked P (some "Favorites") panelGraph (.page, "X") = [("B", 0), ("R", 0)] := by decide +kernel

theorem favorites_exact_key_only :
    linked P (some "F") panelGraph (.page, "A") = [("B", 0), ("Favorites", 0), ("R", 0)] ∧
    linked P none panelGraph (.page, "A") = linked P (some "") panelGraph (.page, "A") ∧
    referenceCount P none panelGraph (.page, "A") = 3 := by decide +kernel

theorem offered_panel_change :
    safe P SP E Witness13.w7 [] "A/x" (literal P Witness13.w7 "A/x" "B/z") = true ∧
    linked P none Witness13.w7 (.journal, "A") = [("R", 0)] ∧
    linked P none (Witness13.after Witness13.w7 "A/x" "B/z") (.journal, "A") = [] ∧
    referenceCount P none (Witness13.after Witness13.w7 "A/x" "B/z") (.journal, "A") = 0 := by
  decide +kernel

def queryGraph : Graph := [pg "R" "header;preamble;root;child;grandchild;other-root"]
def Q : QParams String String := { Witness11.Q with
  levels := fun _ _ => [0, 0, 0, 1, 2, 0]
  preBlock := fun _ _ => true
  blockLeaf := fun s _ b => if s == "gap" then decide (b.2 ≠ 3) else true }
def rows : QueryRowParser := ⟨fun _ _ i => if i == 0 then .header else if i == 1 then .preamble else .body⟩
def allQuery : Query String := .leaf (.given "all")
def gapQuery : Query String := .leaf (.given "gap")
def bn (i : Nat) : Node String := .block ("R", i)

theorem raw_and_default_query_rows :
    den P Q queryGraph .block allQuery = [bn 0, bn 1, bn 2, bn 3, bn 4, bn 5] ∧
    queryResults P Q rows {} queryGraph .block allQuery = [bn 1, bn 2, bn 5] ∧
    queryResults P Q rows ⟨true, true⟩ queryGraph .block allQuery = [bn 0, bn 1, bn 2, bn 5] := by
  decide +kernel

theorem explicit_false_honored :
    queryResults P Q rows ⟨false, false⟩ queryGraph .block allQuery = [bn 1, bn 2, bn 3, bn 4, bn 5] ∧
    queryResults P Q rows ⟨false, true⟩ queryGraph .block allQuery = den P Q queryGraph .block allQuery := by
  decide +kernel

theorem immediate_parent_not_any_ancestor :
    queryParent P Q rows queryGraph ("R", 4) = some ("R", 3) ∧
    den P Q queryGraph .block gapQuery = [bn 0, bn 1, bn 2, bn 4, bn 5] ∧
    queryResults P Q rows {} queryGraph .block gapQuery = [bn 1, bn 2, bn 4, bn 5] := by
  decide +kernel

theorem preblocks_and_page_rows :
    queryParent P Q rows queryGraph ("R", 0) = none ∧
    queryParent P Q rows queryGraph ("R", 1) = none ∧
    queryResults P Q rows {} queryGraph .page allQuery = [.page (.page, "R")] := by
  decide +kernel

theorem offered_query_projection_recomputed :
    safe P SP E queryGraph [] "R" (literal P queryGraph "R" "S") = true ∧
    queryResults P Q rows {} (execute P E queryGraph "R" (literal P queryGraph "R" "S"))
      .block allQuery = [.block ("S", 1), .block ("S", 2), .block ("S", 5)] := by decide +kernel

end PresentationWitness13
end CM
