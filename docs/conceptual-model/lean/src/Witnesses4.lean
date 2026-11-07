/- Round 4: small executable witnesses of exact refusal, configured Favorites,
real-name namespace creation, and the strengthened preservation/W2 boundaries. -/
import Witnesses3

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace Witnesses4
open Params Witnesses3
open Witnesses2 (pg)

theorem journal_child_refused :
    Decided.renameBaseAllowed T JournalChild "old" "new" = true ∧
    Decided.journalRewriteRisks T JournalChild "old" "new" = ["old/sub"] ∧
    Decided.rename T R JournalChild "old" "new" = none := by decide +kernel

def Untouched : Graph := [pg "old" "h", ⟨"old/sub", .journals, .md, "t:old/sub"⟩]
theorem untouched_journal_allowed :
    Decided.journalRewriteRisks T Untouched "old" "new" = [] ∧
    Decided.renameAllowed T Untouched "old" "new" = true ∧
    T.claim (R.apply (nsSubst T.key T.urlHasOrigin "old" "new") ⟨"old/sub", .journals, .md, "t:old/sub"⟩) = (.journal, "old/sub") ∧
    nsSubst T.key T.urlHasOrigin "old" "new" "old/sub" = "new/sub" := by decide +kernel

def Exceptions : Graph := [pg "old" "k:old/sub r:old/sub",
  ⟨"old/sub", .journals, .md, "t:old/sub"⟩]
theorem journal_exceptions_allowed :
    Decided.journalRewriteRisks T Exceptions "old" "new" = [] ∧
    Decided.renameAllowed T Exceptions "old" "new" = true ∧
    T.referencesB (T.rename R.base Exceptions "old" "new") ("new", 0) (.journal, "old/sub") = true :=
  by decide +kernel

def Prefix : Graph := [pg "a/x" "l:a/x/y", ⟨"a", .journals, .md, "t:a"⟩]
theorem journal_prefix_refused :
    nsMatch T.key T.urlHasOrigin "a/x" "a" = none ∧
    Decided.renameBaseAllowed T Prefix "a/x" "b/z" = true ∧
    Decided.journalRewriteRisks T Prefix "a/x" "b/z" = ["a"] ∧
    Decided.rename T R Prefix "a/x" "b/z" = none := by decide +kernel

theorem retained_journal_prefix_allowed :
    Decided.journalRewriteRisks T JournalChild "old" "old/sub/deep" = [] ∧
    Decided.renameAllowed T JournalChild "old" "old/sub/deep" = true ∧
    T.referencesB (T.rename R.base JournalChild "old" "old/sub/deep")
      ("old/sub/deep", 0) (.journal, "old/sub") = true := by decide +kernel

def AliasJournal : Graph := [pg "old" "l:old/sub", ⟨"j", .journals, .md, "a:old/sub"⟩]
theorem journal_alias_policy :
    Decided.journalRewriteRisks T AliasJournal "old" "new" = [] ∧
    Decided.renameAllowed T AliasJournal "old" "new" = true ∧
    Decided.journalRewriteRisks TJ AliasJournal "old" "new" = ["old/sub"] ∧
    Decided.renameBaseAllowed TJ AliasJournal "old" "new" = true ∧
    Decided.rename TJ R AliasJournal "old" "new" = none := by decide +kernel

/- Round 9 (CM-NAMES (2)): a journal's name is its stem, not its title, so the
duplicate journal claim is now two files with the same stem in two formats
(like `journals/j.md` next to `journals/j.org`); was two journals titled `j`. -/
def IgnoredAlias : Graph := [pg "old" "l:old/sub",
  ⟨"j", .journals, .md, "t:j"⟩, ⟨"j", .journals, .org, "t:j a:old/sub"⟩]
theorem journal_nonowner_alias_ignored :
    TJ.isOwner IgnoredAlias ⟨"j", .journals, .org, "t:j a:old/sub"⟩ = false ∧
    Decided.journalRewriteRisks TJ IgnoredAlias "old" "new" = [] ∧
    Decided.renameAllowed TJ IgnoredAlias "old" "new" = true := by decide +kernel

/-- The guard protects per-occurrence spelling support, not whole alias concepts.
An alias-connected journal can still lose a reference with no owned key lost. -/
def AliasConnected : Graph := [pg "old" "h", pg "p" "a:old/sub l:p",
  ⟨"old/sub", .journals, .md, "t:old/sub"⟩]
theorem journal_blanket_still_false :
    T.conflictFreeB AliasConnected = true ∧
    Decided.renameAllowed T AliasConnected "old" "new" = true ∧
    T.referencesB AliasConnected ("p", 0) (.journal, "old/sub") = true ∧
    T.referencesB (T.rename R.base AliasConnected "old" "new") ("p", 0) (.journal, "old/sub") = false :=
  by decide +kernel

def Unreferenced : Graph := [pg "n/y" "h"]
theorem unreferenced_parent :
    T.refKeys Unreferenced = [] ∧ T.nameKeys Unreferenced = ["n"] ∧
    ((.page, "n") : Page String) ∈ Decided.pages T Unreferenced ∧
    ((.page, "n") : Page String) ∉ T.pagesLegacyA Unreferenced ∧
    T.referencesB Unreferenced ("n/y", 0) (.page, "n") = false := by decide +kernel

theorem legacy_universe_equality_false : T.pagesR false Unreferenced ≠ T.pagesLegacyA Unreferenced :=
  by decide +kernel

def NamespaceJournal : Graph := [⟨"n/y", .journals, .org, "t:n/y"⟩]
theorem journal_name_parent : T.nameKeys NamespaceJournal = ["n"] ∧
    ((.page, "n") : Page String) ∈ Decided.pages T NamespaceJournal := by decide +kernel

def NamespaceDuplicates : Graph := [pg "n/y" "", pg "z" "t:n/y"]
theorem nonowner_name_parent :
    T.isOwner NamespaceDuplicates (pg "z" "t:n/y") = false ∧
    T.fileNsNames (pg "n/y" "") = [] ∧
    T.fileNsNames (pg "z" "t:n/y") = ["n"] ∧
    "n" ∈ T.nameKeys NamespaceDuplicates := by decide +kernel

def URLFixture : Params String := { T with urlHasOrigin := fun s => s == "https://host/n/y" }
theorem namespace_exclusions :
    T.fileNsNames (pg "./n/y" "h") = [] ∧
    T.fileNsNames (pg "../n/y" "h") = [] ∧
    URLFixture.fileNsNames (pg "https://host/n/y" "h") = [] ∧
    T.fileNsNames (pg "n/y" "") = [] ∧
    T.fileNsNames (pg "n/y/z" "h") = ["n", "n/y"] ∧
    T.fileNsNames (pg "/n/y" "h") = ["/n"] := by decide +kernel

def NameOnly : Graph := [pg "x" "h"]
def mn : String → String := nsSubst T.key T.urlHasOrigin "x" "n/y"
def NameOnly' : Graph := T.rename R.base NameOnly "x" "n/y"
theorem name_only_hyps : T.MergeHyps R.base mn mn NameOnly where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
    path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
    order := by decide +kernel }
  alias := by decide +kernel
  realInj := by decide +kernel

theorem name_only_allowed : Decided.rename T R NameOnly "x" "n/y" = some NameOnly' ∧
    T.refKeys NameOnly = [] ∧ T.refKeys NameOnly' = [] ∧
    T.nameKeys NameOnly = [] ∧ T.nameKeys NameOnly' = ["n"] ∧
    T.pagesR true NameOnly = [(.page, "x")] ∧
    T.pagesR true NameOnly' = [(.page, "n/y"), (.page, "n")] := by decide +kernel

theorem name_only_old_conditions :
    (∀ f ∈ NameOnly, ∀ t ∈ T.outline f.format f.text, ∀ o ∈ T.occ f.format t,
      R.base.covers o.1 = true →
        (∀ k ∈ (T.occClosure (mn o.2)).map T.key,
          k ∈ (T.occClosure o.2).map (fun u => mn (T.key u))) ∧
        (∀ k ∈ (T.occClosure o.2).map (fun u => mn (T.key u)),
          k ∈ (T.occClosure (mn o.2)).map T.key)) ∧
    (∀ a ∈ T.inPlayKeys NameOnly, ∀ b ∈ T.inPlayKeys NameOnly, mn a = mn b → a = b) ∧
    T.GraphExceptionFree R.base mn NameOnly := by
  unfold GraphExceptionFree ExceptionFree
  decide +kernel

theorem name_only_exact_applies :
    "n" ∈ T.rewrittenPageSeedKeys R.base mn NameOnly ∧
    "n" ∉ T.rewrittenRefKeys R.base mn NameOnly ∧
    ((.page, "n") : Page String) ∈ Decided.pages T NameOnly' := by decide +kernel

theorem name_only_no_surjection : ¬ ∃ σ : Page String → Page String,
    ∀ q ∈ Decided.pages T NameOnly', ∃ p ∈ Decided.pages T NameOnly, σ p = q := by
  rintro ⟨σ, hs⟩
  obtain ⟨p, hp, ep⟩ := hs (.page, "n/y") (by decide +kernel)
  obtain ⟨q, hq, eq⟩ := hs (.page, "n") (by decide +kernel)
  have ha : ∀ p ∈ Decided.pages T NameOnly, p = (.page, "x") := by decide +kernel
  have hne : ((.page, "n/y") : Page String) ≠ (.page, "n") := by decide +kernel
  exact hne (ep.symm.trans ((congrArg σ ((ha p hp).trans (ha q hq).symm)).trans eq))

/-- The added names compatibility is inhabited with actual unclaimed parents,
not only empty name-key lists. The key map also transports those parents. -/
def ParentGood : Graph := [pg "old/sub" "h", pg "source" "l:old/sub"]
def mp : String → String := nsSubst T.key T.urlHasOrigin "old/sub" "new/sub"
def sp : String → String := nsSubst T.key T.urlHasOrigin "old" "new"
theorem parent_good_hyps : T.RenameHyps R.base mp sp ParentGood where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
    path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
    order := by decide +kernel }
  keys := {
    alias := by decide +kernel, ns := by decide +kernel, names := by decide +kernel,
    inj := by decide +kernel, refKind := by decide +kernel }

theorem parent_good_transport :
    Decided.renameAllowed T ParentGood "old/sub" "new/sub" = true ∧
    "old" ∈ T.nameKeys ParentGood ∧
    ((.page, "new") : Page String) ∈ Decided.pages T (renameBy R.base mp ParentGood) := by
  refine ⟨by decide +kernel, by decide +kernel, ?_⟩
  exact pagesR_rename_fwd (r := true) (p := (.page, "old")) parent_good_hyps
    (by unfold GraphExceptionFree ExceptionFree; decide +kernel) (by decide +kernel)

def KindFixture : Params String := { T with refKind := fun k => if k = "new" then .journal else .page }
def KindGraph : Graph := [pg "old/sub" "h"]
def mkind : String → String := nsSubst T.key T.urlHasOrigin "old/sub" "new/sub"
def skind : String → String := nsSubst T.key T.urlHasOrigin "old" "new"
theorem name_parent_kind_needed :
    KindFixture.refKeys KindGraph = [] ∧
    (∀ k ∈ KindFixture.refKeys KindGraph, KindFixture.refKind (skind k) = KindFixture.refKind k) ∧
    ((.page, "old") : Page String) ∈ Decided.pages KindFixture KindGraph ∧
    ((.journal, "new") : Page String) ∈ Decided.pages KindFixture (renameBy R.base mkind KindGraph) ∧
    σp skind (.page, "old") ∉ Decided.pages KindFixture (renameBy R.base mkind KindGraph) := by decide +kernel

def oldProtected (S : List String) (G H : Graph) : List String :=
  S ++ ((G ++ H).filter (fun f => (Isolation.rawKeys T f).any (fun k => decide (k ∈ S)))).map
    (fun f => (T.claim f).2)

theorem old_W2_agreement_insufficient :
    (∀ f, (T.claim f).2 ∈ oldProtected ["n"] Unreferenced [] →
      (f ∈ Unreferenced ↔ f ∈ ([] : Graph))) ∧
    Isolation.agreementB T ["n"] Unreferenced [] = false ∧
    ((.page, "n") : Page String) ∈ Decided.pages T Unreferenced ∧
    ((.page, "n") : Page String) ∉ Decided.pages T [] := by
  refine ⟨?_, by decide +kernel, by decide +kernel, by decide +kernel⟩
  intro f hp
  constructor
  · intro hf
    have he : f = pg "n/y" "h" := List.mem_singleton.mp hf
    subst f
    have hn : (T.claim (pg "n/y" "h")).2 ∉ oldProtected ["n"] Unreferenced [] := by decide +kernel
    exact False.elim (hn hp)
  · simp

def config : Decided.Config := ⟨"favorites"⟩
def Favorites : Graph := [pg "favorites" "l:t", pg "t" "h"]
def MovedFavorites : Graph := T.rename R.base Favorites "favorites" "moved"
theorem configured_favorites_moves :
    Decided.renameConfiguredState T R ⟨Favorites, config, ["favorites"]⟩ "favorites" "moved" =
      some ⟨MovedFavorites, config, ["moved"]⟩ ∧
    Decided.linkB T config Favorites (.page, "favorites") (.page, "t") = false ∧
    Decided.linkB T config MovedFavorites (.page, "moved") (.page, "t") = true := by decide +kernel

def IntoFavorites : Graph := [pg "q" "l:t", pg "t" "h"]
theorem configured_favorites_target :
    Decided.renameAllowed T IntoFavorites "q" "favorites" = true ∧
    Decided.linkB T config IntoFavorites (.page, "q") (.page, "t") = true ∧
    Decided.linkB T config (T.rename R.base IntoFavorites "q" "favorites")
      (.page, "favorites") (.page, "t") = false := by decide +kernel

theorem configured_transport_applies :
    Decided.Link T config Good (.page, "c") (.page, "p") ∧
    Decided.Link T config Good' (.page, "c") (.page, "p") := by
  have hl : Decided.Link T config Good (.page, "c") (.page, "p") :=
    (Decided.linkB_iff T _ _ _ _).mp (by decide +kernel)
  exact ⟨hl, (Decided.link_rename_config good_hyps good_exceptionFree
    (by decide +kernel) (by decide +kernel) (by decide +kernel)).mp hl⟩

end Witnesses4
end CM
