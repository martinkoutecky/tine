/- R14 executable witnesses. All day values are calendar identities, not labels. -/

import AliasRealization14

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace DateWitness13
open Params Consolidated13
open Date13

def P : Inputs String := { Witness13.P with
  stem := fun f => if f.path == "pages/2026-11-02.md" then "2026-11-02"
    else if f.path == "pages/Work.md" then "Work"
    else if f.path == "pages/R.md" then "R"
    else if f.path == "pages/X.md" then "X" else f.path }

def D : Dates String := ⟨fun s =>
  if s == "2026-11-02" || s == "2026_11_02" || s == "Nov 2nd, 2026" then some "2026-11-02"
  else if s == "2027-11-02" || s == "2027_11_02" then some "2027-11-02" else none,
  fun path => if path == "journals/2027_11_02.md" then some "2027-11-02"
    else if path == "journals/2026_11_02.md" then some "2026-11-02" else none⟩
/-- The fixture parser accepts full ISO dates even when labels omit the year.
This supplies operation spellings, without changing display or date-source policy. -/
def namesFor (I : Inputs String) (hk : ∀ s, I.key s = s) : MemberNames I D where
  identifying := fun f => if f.dir = .journals then
    (D.journalDay f.path).getD (I.name f) else I.name f
  identifies := by
    intro f
    by_cases hj : f.dir = .journals
    · by_cases h27 : f.path = "journals/2027_11_02.md"
      · simp [Date13.key,Date13.claim,Date13.target,D,hj,h27,hk]
      · by_cases h26 : f.path = "journals/2026_11_02.md"
        · simp [Date13.key,Date13.claim,Date13.target,D,hj,h27,h26,hk]
        · simp [Date13.key,Date13.claim,Date13.target,D,hj,h27,h26,hk]
    · simp [Date13.key,Date13.claim,Date13.target,hj]

def names : MemberNames P D := namesFor P (fun _ => rfl)
def rawNames : MemberNames Witness13.P D := namesFor Witness13.P (fun _ => rfl)

def f (path text : String) : File := ⟨path,.pages,.md,text⟩
def imported := f "pages/2026-11-02.md" "imported;l:OnlyImported"
def journal : File := ⟨"journals/2026_11_02.md",.journals,.md,"journal;l:OnlyJournal"⟩
def reader := f "pages/R.md" "l:2026-11-02;l:2026_11_02;l:Nov 2nd, 2026"
def both : Graph := [imported,journal,reader]
def fresh : File := {journal with text := ""}
abbrev day : Page String := (.journal,"2026-11-02")
abbrev Q := Witness11.Q

theorem same_day_conflict :
    Date13.claim P D imported = day ∧ Date13.claim P D journal = day ∧
    Date13.conflicted P D both day = true ∧
    Date13.openingFile P D both day = some journal ∧
    (Date13.activeGraph P D both).map File.path = [journal.path,reader.path] ∧
    imported ∈ both := by decide +kernel

theorem all_spellings_reference_one_day :
    Date13.resolve P D both "2026-11-02" = day ∧
    Date13.resolve P D both "2026_11_02" = day ∧
    Date13.resolve P D both "Nov 2nd, 2026" = day ∧
    Date13.linkedAt P D none both "2026-11-02" = [(reader.path,0),(reader.path,1),(reader.path,2)] ∧
    Date13.den P D Q both .page (.leaf (.name "2026_11_02")) = [.page day] := by decide +kernel

theorem imported_reused :
    Date13.openingFile P D [imported,reader] day = some imported ∧
    (ensureJournal P D [imported,reader] day.2 fresh).graph = [imported,reader] ∧
    (ensureJournal P D both day.2 fresh).graph = both ∧
    (ensureJournal P D both day.2 fresh).conflicts = [imported,journal] := by decide +kernel

theorem absent_day_created :
    (ensureJournal P D [reader] day.2 fresh).graph = [fresh,reader] ∧
    (ensureJournal P D [reader] day.2 fresh).created = true ∧
    (ensureJournal P D [fresh,reader] day.2 fresh).created = false := by decide +kernel

/-- F3, independently of the unresolved date-source choice: ordinary -> date,
then ensure, reuses the newly date-named pages file. -/
def probePolicy : DatePolicy String := ⟨fun _ _ => false⟩

/-- A fixture policy, not a selected production answer to M2. -/
theorem f3_rename_then_ensure :
    let G := [f "Foo" "body"]
    let c := Date13.literal Witness13.P D probePolicy G "Foo" "2026-11-02"
    let H := Date13.execute Witness13.P D probePolicy Witness13.E G "Foo" c
    Date13.offers Witness13.P D probePolicy rawNames Witness13.SP Witness13.E G [] "Foo" "2026-11-02" = [c] ∧
    H = [f "2026-11-02" "body"] ∧
    Date13.claim Witness13.P D (f "2026-11-02" "body") = day ∧
    (ensureJournal Witness13.P D H day.2 fresh).graph = H ∧
    (ensureJournal Witness13.P D H day.2 fresh).created = false := by
  decide +kernel

/-- Outside moves have no history. With no overriding date title, the same
bytes now belong to Work, while all three date links still name the virtual day. -/
def moved : File := {journal with path := "pages/Work.md",dir := .pages}
def renamed : File := {imported with path := "pages/Work.md"}
def namedToDate : File := {moved with path := "pages/2026-11-02.md"}

theorem outside_move_outcomes :
    moved.text = journal.text ∧ renamed.text = imported.text ∧
    Date13.claim P D moved = (.page,"Work") ∧
    Date13.claim P D renamed = (.page,"Work") ∧
    Date13.openingFile P D [moved,reader] day = none ∧
    Date13.resolve P D [moved,reader] "2026_11_02" = day ∧
    Date13.referencesB P D [moved,reader] (reader.path,0) (.page,"Work") = false ∧
    Date13.claim P D namedToDate = day ∧
    (ensureJournal P D [namedToDate,reader] day.2 fresh).graph = [namedToDate,reader] := by decide +kernel

theorem outside_title_remains_effective :
    Date13.claim P D (f "pages/Work.md" "t:2026-11-02;body") = day ∧
    Date13.claim P D (f "pages/2026-11-02.md" "t:Work;body") = (.page,"Work") := by decide +kernel

def ambiguousP : Inputs String := { P with stem := fun f =>
  if f.dir == .journals then "11-02" else P.stem f }
def ambiguousNames : MemberNames ambiguousP D := namesFor ambiguousP (fun _ => rfl)

def nextYear : File := {journal with path := "journals/2027_11_02.md"}

theorem approved_label_limitation :
    ambiguousP.name journal = ambiguousP.name nextYear ∧
    Date13.claim ambiguousP D journal ≠ Date13.claim ambiguousP D nextYear ∧
    Date13.openingFile ambiguousP D [journal,nextYear] day = some journal ∧
    Date13.openingFile ambiguousP D [journal,nextYear] (.journal,"2027-11-02") = some nextYear ∧
    Date13.resolve ambiguousP D [journal,nextYear] "11-02" = (.page,"11-02") := by decide +kernel

/-- Path order disagrees with effective-name order. X opens B and has B's panel. -/
def panels : Graph := [f "pages/z.md" "t:A|a:X|l:X",f "pages/a.md" "t:B|a:X|l:X",f "pages/R.md" "l:X"]
theorem alias_panel_follows_opening :
    Date13.resolve P D panels "X" = (.page,"B") ∧
    Date13.panelSelf P D panels (.page,"X") = (.page,"B") ∧
    Date13.linkedAt P D none panels "X" = [("pages/z.md",0),("pages/R.md",0)] ∧
    Date13.linkedAt P D none panels "X" = Date13.linkedAt P D none panels "B" := by decide +kernel

def aliasJournal : File := {journal with text := "a:X"}
def aliasPage := f "aaa/a.md" "t:Owner|a:X"
theorem declarers_have_no_directory_rank :
    Date13.resolve P D [aliasJournal,aliasPage] "X" = (.page,"Owner") := by decide +kernel

/-- Fresh ordinary rename is singleton for every M2 policy. -/
theorem ordinary_onestep :
    Date13.automatic Witness13.P D probePolicy rawNames Witness13.SP Witness13.E [f "Foo" "body"] [] "Foo" "Bar" =
      some [f "Bar" "body"] := by decide +kernel

def emptyReal := f "pages/X.md" ""
theorem real_target_precedes_alias :
    Date13.resolve P D [emptyReal,aliasPage] "X" = (.page,"X") := by decide +kernel

def dateAlias := f "pages/Owner.md" "t:Owner|a:2026-11-02"
theorem date_alias_before_after_ensure :
    Date13.resolve P D [dateAlias,reader] "2026_11_02" = (.page,"Owner") ∧
    (ensureJournal P D [dateAlias,reader] day.2 fresh).created = true ∧
    Date13.resolve P D (ensureJournal P D [dateAlias,reader] day.2 fresh).graph "2026_11_02" = day := by decide +kernel

/-- A malformed input snapshot has inconsistent path/directory metadata.
The reader still accepts it; creation must not overwrite the occupied path. -/
def occupied : File := {fresh with dir := .pages,text := "t:Work;valuable"}
theorem occupied_creation_path_refuses :
    (ensureJournal P D [occupied] day.2 fresh).graph = [occupied] ∧
    (ensureJournal P D [occupied] day.2 fresh).file = none := by decide +kernel

def f3Check (s : String) : Bool :=
  let G := [f "Foo" "body"]
  let c := Date13.literal Witness13.P D probePolicy G "Foo" s
  let H := Date13.execute Witness13.P D probePolicy Witness13.E G "Foo" c
  let withJournal := [f "Foo" "body",journal]
  decide (Date13.offers Witness13.P D probePolicy rawNames Witness13.SP Witness13.E G [] "Foo" s = [c] ∧
    Date13.claim Witness13.P D (f s "body") = day ∧
    (ensureJournal Witness13.P D H day.2 fresh).graph = H ∧
    (Date13.literal Witness13.P D probePolicy withJournal "Foo" s).survivor = some journal)

theorem f3_all_spellings : ["2026-11-02","2026_11_02","Nov 2nd, 2026"].all f3Check = true := by decide +kernel

/-- The verifier's counterexample: two independently selectable day members
of Shared, both displayed as 11-02. These are destinations, not date sources. -/
def shared2026 : File := {journal with text := "a:Shared"}
def shared2027 : File := {nextYear with text := "a:Shared"}
def sharedGroup : Graph := [f "Foo" "body",shared2026,shared2027]

/-- Both fixture extremes of M2 give all three choices and refuse automatic
execution. The wrong ordinary 11-02 identity is never offered. -/
theorem comprehensive_ambiguous_group : ∀ fixed : Bool,
    let M : DatePolicy String := ⟨fun _ _ => fixed⟩
    let c26 := Date13.literal ambiguousP D M sharedGroup "Foo" "2026-11-02"
    let c27 := Date13.literal ambiguousP D M sharedGroup "Foo" "2027-11-02"
    let offered := Date13.offers ambiguousP D M ambiguousNames Witness13.SP Witness13.E sharedGroup [] "Foo" "Shared"
    ambiguousP.name shared2026 = "11-02" ∧ ambiguousP.name shared2027 = "11-02" ∧
    shared2026 ∈ Date13.groupFiles ambiguousP D sharedGroup "Shared" ∧
    shared2027 ∈ Date13.groupFiles ambiguousP D sharedGroup "Shared" ∧
    c26.survivor = some shared2026 ∧ c27.survivor = some shared2027 ∧
    offered = [Date13.literal ambiguousP D M sharedGroup "Foo" "Shared",c26,c27] ∧
    Date13.automatic ambiguousP D M ambiguousNames Witness13.SP Witness13.E sharedGroup [] "Foo" "Shared" = none ∧
    (Date13.literal ambiguousP D M sharedGroup "Foo" "11-02") ∉ offered ∧
    Date13.resolve ambiguousP D sharedGroup "11-02" = (.page,"11-02") := by
  decide +kernel

/-- A chosen member executes the same one substitution at ordinary name,
namespace and query-argument sites; the typed Shared is not a second map. -/
theorem ambiguous_group_choice_executes :
    let c := Date13.literal ambiguousP D probePolicy sharedGroup "Foo"
      (memberSpelling ambiguousP D ambiguousNames shared2026)
    Date13.answer ambiguousP D probePolicy ambiguousNames Witness13.SP Witness13.E sharedGroup [] "Foo" "Shared" (some c) =
      some (Date13.execute ambiguousP D probePolicy Witness13.E sharedGroup "Foo" c) ∧
    (Date13.subst ambiguousP D probePolicy "Foo" c "Foo" = "2026-11-02" ∧
     Date13.subst ambiguousP D probePolicy "Foo" c "Foo/x" = "2026-11-02/x" ∧
     c.destination ≠ "11-02") := by
  decide +kernel

end DateWitness13
end CM
