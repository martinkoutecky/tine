/- Discriminating, kernel-evaluated presentation and unresolved date instances.
Date instances are alternatives, not an adopted production policy. -/
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

/- KEY-DATE: unchanged core boundary is one String→K key for claims, aliases,
references, queries and substitutions, with configured refKind and stem.
No production instance for ordinary recognized dates is selected below. -/
def dateGraph : Graph := [pg "2026-11-06" "ordinary",
  ⟨"2026_11_06.md", .journals, .md, "journal"⟩]
def dateLiteral : Inputs String := { P with
  stem := fun f => if f.dir == .journals then "Nov 6th, 2026" else f.path
  refKind := fun s => if s == "2026-11-06" || s == "Nov 6th, 2026" then .journal else .page
  kindPref := .journal }
def dateFolded : Inputs String := { dateLiteral with
  key := fun s => if s == "2026-11-06" then "Nov 6th, 2026" else s }

theorem literal_date_instance :
    dateLiteral.conflictedB dateGraph "2026-11-06" = false ∧
    (activeGraph dateLiteral dateGraph).map File.path = ["2026-11-06", "2026_11_06.md"] ∧
    resolve dateLiteral dateGraph "2026-11-06" = (.page, "2026-11-06") ∧
    resolve dateLiteral dateGraph "Nov 6th, 2026" = (.journal, "Nov 6th, 2026") := by decide +kernel

theorem folded_date_instance :
    dateFolded.conflictedB dateGraph "Nov 6th, 2026" = true ∧
    (activeGraph dateFolded dateGraph).map File.path = ["2026_11_06.md"] ∧
    resolve dateFolded dateGraph "2026-11-06" = (.journal, "Nov 6th, 2026") := by decide +kernel

/-- A globally date-folded key cannot simultaneously give an ordinary file
a literal, distinct claim for one of those same spellings. -/
theorem shared_key_forces_claim (I : Inputs String) (f : File) (s canonical : String)
    (hn : I.name f = s) (hk : I.key s = I.key canonical) :
    (I.claim f).2 = I.key canonical := by simp [Params.claim, hn, hk]

end PresentationWitness13
end CM
