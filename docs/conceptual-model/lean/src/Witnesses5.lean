/- Finite kernel fixtures for the decided O3/O4 policy. No production parser
claim: dateKey recognises just the two dates used in these fixtures. -/
import Policy5

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace Witnesses5
open Params Witnesses3
open Witnesses2 (pg)

def dateKey (k : String) : Bool := k == "2026-10-26" || k == "2026-10-27"
def D : Params String := Policy5.params T dateKey
def D0 : Params String := Policy5.pageOnly T dateKey
def config : Decided.Config := ⟨"favorites"⟩
theorem lawful : D.Lawful := Policy5.lawful T dateKey Witnesses3.lawful

def DateRef : Graph := [pg "source" "l:2026-10-26"]
theorem date_reference :
    D.resolve DateRef "2026-10-26" = (.journal, "2026-10-26") ∧
    ((.journal, "2026-10-26") : Page String) ∈ Decided.pages D DateRef ∧
    ((.page, "2026-10-26") : Page String) ∉ Decided.pages D DateRef ∧
    Decided.linkB D config DateRef (.page, "source") (.journal, "2026-10-26") = true :=
  by decide +kernel

def DateParent : Graph := [pg "2026-10-26/sub" "h"]
theorem date_parent :
    D.refKeys DateParent = [] ∧ D.nameKeys DateParent = ["2026-10-26"] ∧
    ((.journal, "2026-10-26") : Page String) ∈ Decided.pages D DateParent ∧
    D.pageBlocks DateParent (.journal, "2026-10-26") = [] ∧
    D.referencesB DateParent ("2026-10-26/sub", 0) (.journal, "2026-10-26") = false :=
  by decide +kernel

def DateFile : Graph := [⟨"pages/2026-10-26.md", .pages, .md, "t:2026-10-26"⟩,
  pg "source" "l:2026-10-26"]
theorem date_file_reachable :
    D.resolve DateFile "2026-10-26" = (.page, "2026-10-26") ∧
    D.owner DateFile (.page, "2026-10-26") =
      some ⟨"pages/2026-10-26.md", .pages, .md, "t:2026-10-26"⟩ ∧
    ((.journal, "2026-10-26") : Page String) ∉ Decided.pages D DateFile ∧
    Decided.linkB D config DateFile (.page, "source") (.page, "2026-10-26") = true :=
  by decide +kernel

theorem empty_real_date_wins :
    D.resolve [pg "2026-10-26" ""] "2026-10-26" = (.page, "2026-10-26") :=
  by decide +kernel

def DateClash : Graph := DateFile ++ [⟨"2026-10-26", .journals, .md, "t:2026-10-26"⟩]
def OtherPreference : Params String := { D with kindPref := .journal }
theorem no_new_journal_priority :
    D.conflictedB DateClash "2026-10-26" = true ∧
    D.resolve DateClash "2026-10-26" = (.page, "2026-10-26") ∧
    OtherPreference.resolve DateClash "2026-10-26" = (.journal, "2026-10-26") :=
  by decide +kernel

theorem date_alias_suppressed :
    D.resolve [⟨"j", .journals, .md, "a:2026-10-26"⟩, pg "s" "l:2026-10-26"]
      "2026-10-26" = (.journal, "j") ∧
    ((.journal, "2026-10-26") : Page String) ∉
      Decided.pages D [⟨"j", .journals, .md, "a:2026-10-26"⟩, pg "s" "l:2026-10-26"] :=
  by decide +kernel

def DayAliases : Graph := [⟨"2026-10-26", .journals, .md, "h t:2026-10-26 a:today"⟩,
  pg "source" "l:today"]
theorem journal_alias_counts :
    D.aliasEdges DayAliases = [("2026-10-26", "today")] ∧
    D.resolve DayAliases "today" = (.journal, "2026-10-26") ∧
    ((.page, "today") : Page String) ∉ Decided.pages D DayAliases ∧
    D.referencesB DayAliases ("source", 0) (.journal, "2026-10-26") = true ∧
    D.leafBlock3 Q DayAliases (.refs "2026-10-26") ("source", 0) = true ∧
    Decided.linkB D config DayAliases (.page, "source") (.journal, "2026-10-26") = true ∧
    ((.journal, "2026-10-26") : Page String) ∈
      Decided.nearL D config DayAliases .out 1 [(.page, "source")] := by decide +kernel

theorem O4_stricter_refusal :
    Decided.renameBaseAllowed D0 Witnesses4.AliasJournal "old" "new" = true ∧
    Decided.renameAllowed D0 Witnesses4.AliasJournal "old" "new" = true ∧
    Decided.renameBaseAllowed D Witnesses4.AliasJournal "old" "new" = true ∧
    Decided.journalRewriteRisks D Witnesses4.AliasJournal "old" "new" = ["old/sub"] ∧
    Decided.rename D R Witnesses4.AliasJournal "old" "new" = none := by decide +kernel

/-- O4 also suppresses alias-only parents via Q9. Therefore the WHOLE refusal
is not monotone: no occurrence loses the suppressed parent here. -/
def SuppressedParent : Graph := [pg "old" "h", pg "old/sub/deep" "h",
  pg "new/sub" "h", ⟨"j", .journals, .md, "a:old/sub"⟩]
theorem O4_relaxes_base_guard :
    D.conflictFreeB SuppressedParent = true ∧
    Decided.renameBaseAllowed D0 SuppressedParent "old" "new" = false ∧
    Decided.renameAllowed D0 SuppressedParent "old" "new" = false ∧
    Decided.renameBaseAllowed D SuppressedParent "old" "new" = true ∧
    Decided.journalRewriteRisks D SuppressedParent "old" "new" = [] ∧
    Decided.renameAllowed D SuppressedParent "old" "new" = true := by decide +kernel

theorem direct_guard_boundaries :
    Decided.rename D R JournalChild "old" "new" = none ∧
    Decided.renameAllowed D Witnesses4.Untouched "old" "new" = true ∧
    Decided.renameAllowed D Witnesses4.Exceptions "old" "new" = true ∧
    Decided.renameAllowed D JournalChild "old" "old/sub/deep" = true ∧
    Decided.renameAllowed D Witnesses4.IgnoredAlias "old" "new" = true := by decide +kernel

theorem journal_blanket_refuted :
    D.conflictFreeB Witnesses4.AliasConnected = true ∧
    Decided.renameAllowed D Witnesses4.AliasConnected "old" "new" = true ∧
    D.referencesB Witnesses4.AliasConnected ("p", 0) (.journal, "old/sub") = true ∧
    D.referencesB (D.rename R.base Witnesses4.AliasConnected "old" "new")
      ("p", 0) (.journal, "old/sub") = false := by decide +kernel

def crossDate : String → String := nsSubst D.key D.urlHasOrigin "2026-10-26/sub" "ordinary/sub"
def parentMap : String → String := nsSubst D.key D.urlHasOrigin "2026-10-26" "ordinary"
theorem date_kind_condition_needed :
    Decided.renameAllowed D DateParent "2026-10-26/sub" "ordinary/sub" = true ∧
    Decided.journalRewriteRisks D DateParent "2026-10-26/sub" "ordinary/sub" = [] ∧
    ((.journal, "2026-10-26") : Page String) ∈ Decided.pages D DateParent ∧
    ((.page, "ordinary") : Page String) ∈ Decided.pages D (renameBy R.base crossDate DateParent) ∧
    σp parentMap (.journal, "2026-10-26") ∉ Decided.pages D (renameBy R.base crossDate DateParent) ∧
    D.refKind (parentMap "2026-10-26") ≠ D.refKind "2026-10-26" := by decide +kernel

theorem merge_allowed :
    Decided.rename D R Merge "a" "b" = some Merge' ∧ D.conflictFreeB Merge = true :=
  by decide +kernel

theorem T3_bijection_refuted : ¬ ∃ σ, Witnesses2.PageBij σ
    (Decided.pages D Merge) (Decided.pages D Merge') := by
  exact Witnesses2.no_bij_lt (A₀ := [(.page, "a"), (.page, "b")])
    (B₀ := [(.page, "b")]) (by decide +kernel) (by decide +kernel)
    (by decide +kernel) (by decide +kernel)

theorem T3_surjection_refuted :
    Decided.renameAllowed D Witnesses4.NameOnly "x" "n/y" = true ∧
    ¬ ∃ σ : Page String → Page String,
      ∀ q ∈ Decided.pages D Witnesses4.NameOnly',
        ∃ p ∈ Decided.pages D Witnesses4.NameOnly, σ p = q := by
  refine ⟨by decide +kernel, ?_⟩
  have h1 : Decided.pages D Witnesses4.NameOnly = Decided.pages T Witnesses4.NameOnly :=
    by decide +kernel
  have h2 : Decided.pages D Witnesses4.NameOnly' = Decided.pages T Witnesses4.NameOnly' :=
    by decide +kernel
  rw [h1, h2]
  exact Witnesses4.name_only_no_surjection

theorem T6_preservation_refuted :
    Decided.linkB D config Merge (.page, "a") (.page, "b") = true ∧
    Decided.linkB D config Merge' (.page, "b") (.page, "b") = false := by decide +kernel

theorem T3_image_refuted :
    Decided.renameAllowed D AliasMerge "a" "b" = true ∧
    D.conflictFreeB AliasMerge = true ∧
    D.refs AliasMerge' ("b", 0) ≠
      (fun C => ∃ C₀, D.refs AliasMerge ("a", 0) C₀ ∧ C = img mm C₀) := by
  refine ⟨by decide +kernel, by decide +kernel, ?_⟩
  intro heq
  have hr : D.refs AliasMerge' ("b", 0) (D.concept AliasMerge' "b") :=
    ⟨"b", by decide +kernel, rfl⟩
  rw [heq] at hr
  obtain ⟨C₀, ⟨s, hs, rfl⟩, hc⟩ := hr
  have hnames : D.refNames AliasMerge ("a", 0) = ["b"] := by decide +kernel
  rw [hnames, List.mem_singleton] at hs
  subst s
  have hx : D.concept AliasMerge' "b" "x" :=
    (D.conceptB_iff _ _ _).mp (by decide +kernel)
  obtain ⟨k, hk, he⟩ := (congrFun hc "x").mp hx
  rw [D.concept_eq_class] at hk
  have isolated : ∀ e ∈ D.aliasEdges AliasMerge, e.1 ≠ "b" ∧ e.2 ≠ "b" :=
    by decide +kernel
  have hkb : k = "b" := (cls_isolated isolated k).mp hk
  rw [hkb] at he
  exact (show mm "b" ≠ "x" from by decide +kernel) he

theorem Q619_still_nonuniform :
    D.kindUniformB3 Q true DayAliases (.refs "today") = false ∧
    D.kindUniformB3 Q true DayAliases (.page (.refs "today")) = true := by decide +kernel

theorem W1_W2_nonempty :
    D.conflictKeys Bad = ["dup"] ∧ D.conflictFreeB Repaired = true ∧
    Isolation.agreementB D region Bad Repaired = true ∧
    D.resolve Bad "aka" = (.page, "safe") ∧ D.resolve Repaired "aka" = (.page, "safe") :=
  by decide +kernel

def Positive : Graph := [pg "old" "l:today",
  ⟨"2026-10-26", .journals, .md, "h t:2026-10-26 a:today"⟩]
def pm : String → String := nsSubst D.key D.urlHasOrigin "old" "new"
theorem positive_hyps : D.RenameHyps R.base pm pm Positive where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
    path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
    order := by decide +kernel }
  keys := {
    alias := by decide +kernel, ns := by decide +kernel, names := by decide +kernel,
    inj := by decide +kernel, refKind := by decide +kernel }

theorem positive_transport :
    Decided.rename D R Positive "old" "new" = some (renameBy R.base pm Positive) ∧
    Decided.Link D config Positive (.page, "old") (.journal, "2026-10-26") ∧
    Decided.Link D config (renameBy R.base pm Positive) (.page, "new") (.journal, "2026-10-26") := by
  have hl : Decided.Link D config Positive (.page, "old") (.journal, "2026-10-26") :=
    (Decided.linkB_iff D _ _ _ _).mp (by decide +kernel)
  refine ⟨by decide +kernel, hl, ?_⟩
  exact (Decided.link_rename_config positive_hyps
    (by unfold GraphExceptionFree ExceptionFree; decide +kernel)
    (by decide +kernel) (by decide +kernel) (by decide +kernel)).mp hl

end Witnesses5
end CM
