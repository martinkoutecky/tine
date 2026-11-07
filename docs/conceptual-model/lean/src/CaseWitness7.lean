/- Round 7 witnesses for the case-only rename repair (D-20261102-CM4).
TC is the round-3 toy parser with a case-folding key, so `Foo` and `foo`
name the same page. -/
import CaseRelabel7

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace CaseWitness7
open Params
open Witnesses2 (pg)

def lower (s : String) : String := String.ofList (s.toList.map Char.toLower)
def TC : Params String := { Witnesses3.T with key := lower }
def R : Decided.Rewriter := Witnesses3.R

def Case : Graph := [pg "Foo" "l:bar", pg "bar" "l:FOO;l:Foo/x", pg "Foo/x" ""]
def Case' : Graph := [pg "foo" "l:bar", pg "bar" "l:foo;l:foo/x", pg "foo/x" ""]
/- Round 9 (CM-NAMES (2)): the journal is named by its stem, so the journal
`Foo/y` now sits at stem `Foo/y` (round 8: stem `j` with title `Foo/y`). -/
def Jr : Graph := [pg "Foo" "", ⟨"Foo/y", .journals, .md, "t:Foo/y"⟩, pg "c" "l:Foo/y"]
def CRc : Graph := [pg "Foo" "a:Bar", pg "Bar" ""]

theorem lawful : TC.Lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
def mc : String → String := nsSubst TC.key TC.urlHasOrigin "Foo" "foo"

/-- **CW1.** `Foo` → `foo` is accepted by the round-7 guard; the round-6 guard
and the historical SPEC §5 precondition both refused it. -/
theorem case_allowed : Decided.renameAllowed TC Case "Foo" "foo" = true ∧
    CaseRename7.renameAllowedR6 TC Case "Foo" "foo" = false ∧
    TC.renameAllowedLegacy Case "Foo" "foo" = false := by decide +kernel

/-- **CW2.** The after-graph: the file, its namespace child and every covered
spelling now read `foo`; the page universe is unchanged. -/
theorem case_after : Decided.rename TC R Case "Foo" "foo" = some Case' ∧ Case ≠ Case' ∧
    Decided.pages TC Case' = Decided.pages TC Case ∧
    Decided.pages TC Case = [(.page, "foo"), (.page, "bar"), (.page, "foo/x")] := by decide +kernel

theorem case_clean : TC.conflictFreeB Case = true := by decide +kernel

/-- T3 does not apply to the real rewriter with σ = id: the file moves
(`RwLaws.path` fails). -/
theorem case_moves_file : (R.base.apply mc (pg "Foo" "l:bar")).path = "foo" := by decide +kernel

/-- The path-frozen rewriter satisfies T3's hypotheses with σ = id. -/
theorem case_frozen_hyps : TC.RenameHyps (CaseRelabel7.keepPath R.base) mc id Case where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  keys := {
    alias := by decide +kernel
    names := by decide +kernel, ns := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

/-- **CW3.** Through CM-MOVE: the page universe is unchanged, every block keeps
exactly its references at its new id, every page keeps its blocks. -/
theorem case_transport :
    (∀ (r : Bool) (p : Page String),
      p ∈ TC.pagesR r (renameBy R.base mc Case) ↔ p ∈ TC.pagesR r Case) ∧
    (∀ b ∈ TC.blocks Case, ∀ t : Page String, TC.InPlay Case t.2 →
      (TC.References Case b t ↔
        TC.References (renameBy R.base mc Case) (blockMap R.base mc Case b) t)) ∧
    (∀ p : Page String, TC.InPlay Case p.2 →
      TC.pageBlocks (renameBy R.base mc Case) p = (TC.pageBlocks Case p).map (blockMap R.base mc Case)) :=
  CaseRelabel7.moving_transport TC case_frozen_hyps (by unfold WF; decide +kernel)
    (by decide +kernel) (by decide +kernel) (by decide +kernel) (by decide +kernel)

theorem case_after_is_rename : renameBy R.base mc Case = Case' := by decide +kernel

theorem case_refs_example : TC.References Case ("bar", 0) (.page, "foo") ∧
    TC.References Case' ("bar", 0) (.page, "foo") := by
  constructor <;> exact TC.referencesB_iff _ _ _ |>.mp (by decide +kernel)

/-- **CW4.** The real-target clause no longer counts a page as colliding with
itself: a journal named `Foo/y` (under the moved subtree, but not a page file)
blocked the round-6 case-only rename; its key is unchanged, so round 7 accepts. -/
theorem case_journal_title : Decided.renameAllowed TC Jr "Foo" "foo" = true ∧
    CaseRename7.renameAllowedR6 TC Jr "Foo" "foo" = false := by decide +kernel

/-- **CW5.** The alias-class clause still refuses a different real page of the
class (`Foo` aliases `Bar`; renaming to `bar`), while the case-only rename of the
same source is accepted. The round-2 witness CR keeps its refusal too. -/
theorem class_refusal_kept : Decided.renameAllowed TC CRc "Foo" "bar" = false ∧
    Decided.classClash TC CRc "Foo" "bar" = true ∧
    Decided.renameAllowed TC CRc "Foo" "FOO" = true ∧
    Decided.renameAllowed Witnesses2.T Witnesses2.CR "a" "a/b" = false ∧
    Decided.classClash Witnesses2.T Witnesses2.CR "a" "a/b" = true := by decide +kernel

end CaseWitness7
end CM
