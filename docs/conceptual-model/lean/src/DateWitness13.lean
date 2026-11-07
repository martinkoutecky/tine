/- Kernel-executed date, conflict, alias and candidate witnesses. -/
import Date13

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace DateWitness13
open Params Consolidated13
open Date13 (Dates)
open Witnesses2 (pg)

def P : Inputs String := { Witness13.P with
  stem := fun f => if f.dir == .journals then "Nov 6th, 2026" else f.path
  refKind := fun _ => .page }
def D : Dates String := ⟨fun s =>
  if s == "2026-11-06" || s == "2026_11_06" || s == "Nov 6th, 2026" then some "Nov 6th, 2026" else none⟩
abbrev Q := Witness11.Q
abbrev E := Witness13.E
abbrev SP := Witness13.SP
def ordinary := pg "2026-11-06" "ordinary"
def journal : File := ⟨"journals/2026_11_06.md", .journals, .md, "journal"⟩
def reader := pg "R" "l:2026-11-06;l:2026_11_06"
def both : Graph := [ordinary, journal, reader]

theorem coexist_and_open :
    P.conflictedB both "2026-11-06" = false ∧
    P.conflictedB both "Nov 6th, 2026" = false ∧
    (activeGraph P both).map File.path = ["2026-11-06", "journals/2026_11_06.md", "R"] ∧
    Date13.resolve P D both "2026-11-06" = (.page, "2026-11-06") ∧
    Date13.resolve P D both "2026_11_06" = (.journal, "Nov 6th, 2026") ∧
    Date13.resolve P D both "Nov 6th, 2026" = (.journal, "Nov 6th, 2026") := by decide +kernel

theorem references_separate :
    Date13.referencesB P D both ("R", 0) (.page, "2026-11-06") = true ∧
    Date13.referencesB P D both ("R", 0) (.journal, "Nov 6th, 2026") = false ∧
    Date13.referencesB P D both ("R", 1) (.journal, "Nov 6th, 2026") = true ∧
    Date13.referencesB P D both ("R", 1) (.page, "2026-11-06") = false ∧
    Date13.linked P D none both (.page, "2026-11-06") = [("R", 0)] ∧
    Date13.linked P D none both (.journal, "Nov 6th, 2026") = [("R", 1)] := by decide +kernel

theorem raw_panel_has_literal_self :
    Date13.panelSelf P D both (Date13.target P D both "2026-11-06") = (.page, "2026-11-06") ∧
    Date13.panelSelf P D both (Date13.target P D both "2026_11_06") = (.journal, "Nov 6th, 2026") ∧
    Date13.linkedAt P D none both "2026-11-06" = [("R", 0)] ∧
    Date13.linkedAt P D none both "2026_11_06" = [("R", 1)] := by decide +kernel

theorem virtual_day_and_query :
    Date13.resolve P D [ordinary, reader] "2026_11_06" = (.journal, "Nov 6th, 2026") ∧
    (.journal, "Nov 6th, 2026") ∈ Date13.pages P D [ordinary, reader] ∧
    Date13.den P D Q both .page (.leaf (.name "2026_11_06")) = [.page (.journal, "Nov 6th, 2026")] ∧
    Date13.den P D Q both .page (.leaf (.name "2026-11-06")) = [.page (.page, "2026-11-06")] := by decide +kernel

/-- The canonical-title corner cannot be represented by a single kindless key. -/
def canonicalOrdinary := pg "Nov 6th, 2026" "ordinary"
def canonicalReader := pg "R" "l:Nov 6th, 2026;l:2026_11_06"
def canonicalOnly : Graph := [canonicalOrdinary, canonicalReader]
theorem canonical_virtual_separate :
    Date13.resolve P D canonicalOnly "Nov 6th, 2026" = (.page, "Nov 6th, 2026") ∧
    Date13.resolve P D canonicalOnly "2026_11_06" = (.journal, "Nov 6th, 2026") ∧
    Date13.referencesB P D canonicalOnly ("R", 1) (.page, "Nov 6th, 2026") = false ∧
    Date13.referencesB P D canonicalOnly ("R", 1) (.journal, "Nov 6th, 2026") = true ∧
    (.page, "Nov 6th, 2026") ∈ Date13.pages P D canonicalOnly ∧
    (.journal, "Nov 6th, 2026") ∈ Date13.pages P D canonicalOnly := by decide +kernel

theorem exact_crosskind_conflict :
    P.conflictedB [canonicalOrdinary, journal] "Nov 6th, 2026" = true ∧
    openingFile P [canonicalOrdinary, journal] "Nov 6th, 2026" = some canonicalOrdinary ∧
    openingFile { P with kindPref := .journal } [canonicalOrdinary, journal] "Nov 6th, 2026" = some journal ∧
    Date13.target { P with kindPref := .journal } D [canonicalOrdinary, journal] "Nov 6th, 2026" =
      (.journal, "Nov 6th, 2026") ∧
    safe P SP E [canonicalOrdinary, journal] [] "Nov 6th, 2026"
      (literal P [canonicalOrdinary, journal] "Nov 6th, 2026" "Z") = false := by decide +kernel

/-- Explicit aliases still join concepts; literal-name queries do not route. -/
def aliased : Graph := [pg "2026-11-06" "a:2026_11_06", journal, reader]
theorem aliases_are_explicit :
    Date13.referencesB P D aliased ("R", 1) (.page, "2026-11-06") = true ∧
    Date13.resolve P D aliased "2026-11-06" = (.page, "2026-11-06") ∧
    Date13.den P D Q aliased .page (.leaf (.name "2026_11_06")) = [.page (.journal, "Nov 6th, 2026")] := by decide +kernel

/-- Renaming S creates the ordinary date page and changes the meaning of the
unedited raw date link. The new preview must include that block. -/
def creating : Graph := [pg "S" "source", journal, pg "R" "l:2026-11-06"]
def candidate := literal P creating "S" "2026-11-06"
def created := execute P E creating "S" candidate
theorem offered_meaning_change_previewed :
    candidate ∈ Date13.offers P D SP E creating [] "S" "2026-11-06" ∧
    Date13.referencesB P D creating ("R", 0) (.journal, "Nov 6th, 2026") = true ∧
    Date13.referencesB P D created ("R", 0) (.journal, "Nov 6th, 2026") = false ∧
    Date13.referencesB P D created ("R", 0) (.page, "2026-11-06") = true ∧
    (("R", 0), ("R", 0)) ∈ Date13.preview P D E creating "S" candidate := by decide +kernel

/-- The accepted non-date presentation witnesses also execute through the
final interpretation, including self fallback, Favorites and query projection. -/
theorem retained_presentations :
    Date13.linked Witness13.P {} (some "Favorites") PresentationWitness13.panelGraph (.page, "A") = [("B", 0), ("R", 0)] ∧
    Date13.linked Witness13.P {} (some "Favorites") PresentationWitness13.panelGraph (.page, "B") = [("A", 0), ("A", 1), ("R", 0)] ∧
    Date13.panelSelf Witness13.P {} PresentationWitness13.panelGraph (.page, "X") = (.page, "A") ∧
    Date13.referenceCount Witness13.P {} (some "F") PresentationWitness13.panelGraph (.page, "A") = 3 ∧
    Date13.queryResults Witness13.P {} PresentationWitness13.Q PresentationWitness13.rows {}
      PresentationWitness13.queryGraph .block PresentationWitness13.gapQuery =
        [PresentationWitness13.bn 1, PresentationWitness13.bn 2, PresentationWitness13.bn 4, PresentationWitness13.bn 5] := by decide +kernel

end DateWitness13
end CM
