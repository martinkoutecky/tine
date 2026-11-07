/- Kernel executable examples of CM12. Tokens use | within a block and ;
between blocks, as in Witness11. D's date extractor recognises two fixture
dates; this is not an implementation of localized journal parsing. -/
import Rulings12

set_option maxRecDepth 8192
set_option maxHeartbeats 3200000
namespace CM
namespace Witness12
open Params Merge7 Rulings9 Ref10 Rulings11 Rulings12 QueryRename6
open Witnesses2 (pg)
open Witness11 (D R J SP Q lr)

def O : File := pg "Machine Learning" "a:ML"
def C : File := pg "c" "l:ML|l:ML|e:ML|v:ML;e:ML"
def G : Graph := [O, C]
def GA : Graph := [pg "Machine Learning" "a:AI", pg "c" "l:AI|l:AI|e:AI|v:AI;e:AI"]
def m : String → String := nsSubst D.key D.urlHasOrigin "ML" "AI"

/-- Counterexample to r11 unsupported_no_mutation and the last conjunct of
solid_target_no_dialog; all three OLD premises of that theorem still hold. -/
theorem old_unsupported_counterexample :
    mergeSource D G "ML" = none ∧ refSource D G "ML" = false ∧
    Decided.renameAllowed D G "ML" "AI" = false ∧
    sourceKind D G "ML" = .aliasOnly ∧
    offers D G "ML" "AI" = [.rename, .cancel] ∧
    answer D R J G "ML" "AI" .rename = some GA ∧ GA ≠ G := by decide +kernel

theorem unused_full :
    D.aliasEdges GA = [("Machine Learning", "AI")] ∧
    D.resolve GA "AI" = (.page, "Machine Learning") ∧
    ((.page, "AI") : Page String) ∉ Decided.pages D GA ∧
    lr G "Machine Learning" = [("c", 0)] ∧
    lr GA "Machine Learning" = [("c", 0)] ∧ lr GA "ML" = [] ∧
    (lr GA "AI").length = 1 ∧
    storedAt D SP GA ("c", 0) = [.leaf (.page (.name "AI")), .leaf (.refs "AI")] ∧
    D.leafBlock3 Q GA (.refs "AI") ("c", 0) = true ∧
    D.leafBlock3 Q GA (.page (.name "AI")) ("c", 0) = false := by decide +kernel

/-- A stored literal page-name query on an alias has no alias page to select;
syntax follows the rename but inventing a page/query result is forbidden by Q9. -/
theorem unused_query_boundary :
    D.leafBlock3 Q G (.page (.name "ML")) ("c", 0) = false ∧
    D.leafBlock3 Q GA (.page (.name "AI")) ("c", 0) = false ∧
    answer D R J G "ML" "AI" .ownPage = none ∧
    answer D R J G "ML" "AI" .cancel = some G := by decide +kernel

def GT : Graph := [O, pg "AI" "body", C, pg "ML/x" "l:ML"]
def GTA : Graph := [pg "Machine Learning" "a:AI", pg "AI" "body",
  pg "c" "l:AI|l:AI|e:AI|v:AI;e:AI", pg "AI/x" "l:AI"]

theorem real_target :
    offers D GT "ML" "AI" = [.mergeOwner, .cancel] ∧
    answer D R J GT "ML" "AI" .rename = none ∧
    answer D R J GT "ML" "AI" .mergeOwner = some GTA ∧
    D.resolve GTA "AI" = (.page, "AI") ∧
    D.aliasEdges GTA = [("Machine Learning", "AI")] ∧
    lr GTA "AI" = [("c", 0), ("AI/x", 0)] ∧
    lr GTA "Machine Learning" = [("c", 0), ("AI/x", 0)] ∧
    D.leafBlock3 Q GTA (.page (.name "AI")) ("AI", 0) = true := by decide +kernel

theorem old_merge_structure_counterexample :
    mergeChosen D R J GT "ML" "AI" = some GTA ∧
    refSource D GT "ML" = false ∧ mergeSource D GT "ML" = none := by decide +kernel

def GS : Graph := [pg "Machine Learning" "a:ML|a:AI", C]
def GSA : Graph := [pg "Machine Learning" "a:AI|a:AI",
  pg "c" "l:AI|l:AI|e:Machine Learning|v:Machine Learning;e:Machine Learning"]

theorem same_owner_alias_target :
    offers D GS "ML" "AI" = [.mergeOwner, .cancel] ∧
    answer D R J GS "ML" "AI" .mergeOwner = some GSA ∧
    answer D R J GS "ML" "AI" .ownPage = none ∧
    D.resolve GSA "AI" = (.page, "Machine Learning") ∧
    (lr GSA "Machine Learning").length = 1 ∧
    D.leafBlock3 Q GSA (.page (.name "Machine Learning")) ("Machine Learning", 0) = true := by
  decide +kernel

def GD : Graph := [O, pg "Artificial Intelligence" "a:AI", C, pg "ML/x" "body"]
def GDA : Graph := [pg "Machine Learning" "a:AI", pg "Artificial Intelligence" "a:AI",
  pg "c" "l:AI|l:AI|e:Artificial Intelligence|v:Artificial Intelligence;e:Artificial Intelligence",
  pg "AI/x" "body"]

theorem other_owner_alias_target :
    offers D GD "ML" "AI" = [.mergeOwner, .cancel] ∧
    answer D R J GD "ML" "AI" .mergeOwner = some GDA ∧
    (lr GDA "Machine Learning").length = 1 ∧
    (lr GDA "Artificial Intelligence").length = 1 ∧
    D.resolve GDA "AI" = (.page, "Artificial Intelligence") ∧
    storedAt D SP GDA ("c", 1) = [.leaf (.page (.name "Artificial Intelligence"))] := by
  decide +kernel

def GF : Graph := G ++ [pg "linked" "l:AI"]
def GFA : Graph := GA ++ [pg "linked" "l:AI"]
theorem reference_only_target :
    ((.page, "AI") : Page String) ∈ Decided.pages D GF ∧
    offers D GF "ML" "AI" = [.rename, .cancel] ∧
    answer D R J GF "ML" "AI" .rename = some GFA ∧
    ((.page, "AI") : Page String) ∉ Decided.pages D GFA ∧
    lr GFA "Machine Learning" = [("c", 0), ("linked", 0)] := by decide +kernel

def GE : Graph := [O, pg "Artificial Intelligence" "a:AI", pg "AI" "", C]
def GEA : Graph := [pg "Machine Learning" "a:AI", pg "Artificial Intelligence" "a:AI", pg "AI" "",
  pg "c" "l:AI|l:AI|e:Artificial Intelligence|v:Artificial Intelligence;e:Artificial Intelligence"]
theorem empty_alias_target :
    offers D GE "ML" "AI" = [.mergeOwner, .cancel] ∧
    answer D R J GE "ML" "AI" .mergeOwner = some GEA ∧
    D.contentFree (pg "AI" "") = true ∧
    D.resolve GEA "AI" = (.page, "Artificial Intelligence") ∧
    ((.page, "AI") : Page String) ∈ Decided.pages D GEA ∧
    (lr GEA "AI").length = 1 := by decide +kernel

def GB : Graph := [O, pg "other" "a:ML", C]
def GBA : Graph := [pg "Machine Learning" "a:AI", pg "other" "a:AI",
  pg "c" "l:AI|l:AI|e:AI|v:AI;e:AI"]
theorem two_source_declarers :
    aliasSource D GB "ML" = true ∧
    offers D GB "ML" "AI" = [.rename, .cancel] ∧
    answer D R J GB "ML" "AI" .rename = some GBA ∧
    D.aliasEdges GBA = [("Machine Learning", "AI"), ("other", "AI")] ∧
    (lr GBA "other").length = 1 ∧ (lr GBA "Machine Learning").length = 1 := by decide +kernel

theorem two_target_declarers :
    offers D (GB ++ [pg "one" "a:AI", pg "two" "a:AI"]) "ML" "AI" = [.cancel] ∧
    offers D (GB ++ [pg "one" "a:AI", pg "two" "a:AI", pg "AI" ""]) "ML" "AI" = [.cancel] := by
  decide +kernel

def GN : Graph := [pg "owner" "a:old/sub", pg "old/sub/child" "body",
  pg "links" "l:old/sub|l:old/sub/ref|e:old/sub|e:old/sub/child"]
def GNA : Graph := [pg "owner" "a:new/sub", pg "new/sub/child" "body",
  pg "links" "l:new/sub|l:new/sub/ref|e:new/sub|e:new/sub/child"]
theorem namespace_source :
    offers D GN "old/sub" "new/sub" = [.rename, .cancel] ∧
    answer D R J GN "old/sub" "new/sub" .rename = some GNA ∧
    D.aliasEdges GNA = [("owner", "new/sub")] ∧
    lr GNA "owner" = [("links", 0)] ∧ lr GNA "new" = [("links", 0)] ∧
    treeParent D id GNA (.page, "new/sub/child") = none ∧
    ((.page, "new/sub") : Page String) ∉ treeNodes D GNA ∧
    ((.page, "new") : Page String) ∈ treeNodes D GNA := by decide +kernel

theorem own_child_guards :
    offers D GN "old/sub" "old/sub/child" = [.cancel] ∧
    offers D (GN ++ [pg "o2" "a:old/sub/alias"]) "old/sub" "old/sub/alias" = [.cancel] ∧
    offers D G "ML" "ML/unused" = [.rename, .cancel] ∧
    answer D R J G "ML" "ML/unused" .rename = some
      [pg "Machine Learning" "a:ML/unused",
       pg "c" "l:ML/unused|l:ML/unused|e:ML/unused|v:ML/unused;e:ML/unused"] := by decide +kernel

theorem namespace_test_shared :
    offers D [pg "owner" "a:.", pg "./x" "body", pg "c" "l:./x|l:."] "." "q" = [.rename, .cancel] ∧
    answer D R J [pg "owner" "a:.", pg "./x" "body", pg "c" "l:./x|l:."] "." "q" .rename =
      some [pg "owner" "a:q", pg "./x" "body", pg "c" "l:./x|l:q"] := by decide +kernel

def DK : Params String := { D with key := String.toLower }
theorem same_key_alias :
    offers DK G "ML" "ml" = [.rename, .cancel] ∧
    answer DK R J G "ML" "ml" .rename = some
      [pg "Machine Learning" "a:ml", pg "c" "l:ml|l:ml|e:ml|v:ml;e:ml"] := by decide +kernel

def JR : Graph := [O, ⟨"2026-10-26", .journals, .md, "a:ML/sub"⟩, pg "links" "l:ML/sub"]
theorem journal_loss_refused :
    Decided.journalRewriteRisks D JR "ML" "AI" = ["ML/sub"] ∧
    offers D JR "ML" "AI" = [.cancel] ∧
    offers D (JR ++ [pg "AI" "body"]) "ML" "AI" = [.cancel] ∧
    offers D [⟨"2026-10-26", .journals, .md, "a:ML"⟩, C] "ML" "AI" = [.cancel] := by decide +kernel

theorem journal_alias_without_occurrence :
    offers D [⟨"2026-10-26", .journals, .md, "a:ML"⟩] "ML" "AI" = [.rename, .cancel] ∧
    answer D R J [⟨"2026-10-26", .journals, .md, "a:ML"⟩] "ML" "AI" .rename =
      some [⟨"2026-10-26", .journals, .md, "a:AI"⟩] := by decide +kernel

theorem date_spelled_alias :
    offers D [pg "owner" "a:2026-10-26", pg "links" "l:2026-10-26|e:2026-10-26"]
      "2026-10-26" "AI" = [.rename, .cancel] ∧
    answer D R J [pg "owner" "a:2026-10-26", pg "links" "l:2026-10-26|e:2026-10-26"]
      "2026-10-26" "AI" .rename = some [pg "owner" "a:AI", pg "links" "l:AI|e:AI"] ∧
    offers D [O, pg "owner" "a:2026-10-26", C] "ML" "2026-10-26" = [.mergeOwner, .cancel] ∧
    offers D [O, ⟨"2026-10-26", .journals, .md, "body"⟩, C]
      "ML" "2026-10-26" = [.mergeOwner, .cancel] := by decide +kernel

theorem conflict_and_collision :
    offers D (GN ++ [pg "new/sub/child" "other"]) "old/sub" "new/sub" = [.cancel] ∧
    offers D (G ++ [pg "AI" "body", pg "other-path" "t:AI"]) "ML" "AI" = [.cancel] := by
  decide +kernel

/-! Non-vacuous instances of the existing transport laws, actually used below. -/

theorem unused_hyps : D.MergeHyps (R.at m).base m m G where
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
  alias := by decide +kernel
  realInj := by decide +kernel

theorem unused_group_guarantee :
    cls (D.aliasEdges GA) "Machine Learning" "AI" := by
  have he : GA = renameBy (R.at m).base m G := by decide +kernel
  rw [he, alias_group_exact unused_hyps]
  exact Reach.fwd (Reach.refl _) (by decide +kernel)

theorem unused_count_guarantee :
    (linkedBlocks D GA (.page, "Machine Learning")).length =
      ((D.blocks G).filter (fun b => predictedRefs D (R.at m).base m m G b (.page, "Machine Learning"))).length := by
  have he : GA = renameBy (R.at m).base m G := by decide +kernel
  rw [he]; exact counts_exact unused_hyps _

theorem unused_stored_law : StoredLaw D SP (R.at m).base m m G := by
  unfold StoredLaw; decide +kernel

theorem unused_stored_guarantee (i : Nat) :
    storedAt D SP GA ("c", i) = (storedAt D SP G ("c", i)).map (renameQuery m) := by
  have he : GA = renameBy (R.at m).base m G := by decide +kernel
  rw [he]
  exact stored_queries_exact SP unused_hyps unused_stored_law (f := C) (by decide +kernel) i

def mq : String → String := queryMap D "ML" "AI" (pg "Artificial Intelligence" "a:AI")
theorem other_owner_hyps : D.MergeHyps (R.at mq).base m m GD where
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
  alias := by decide +kernel
  realInj := by decide +kernel

theorem other_owner_stored_law : StoredLaw D SP (R.at mq).base m mq GD := by
  unfold StoredLaw; decide +kernel

theorem other_owner_stored_guarantee (i : Nat) :
    storedAt D SP GDA ("c", i) = (storedAt D SP GD ("c", i)).map (renameQuery mq) := by
  have he : GDA = renameBy (R.at mq).base m GD := by decide +kernel
  rw [he]
  exact stored_queries_exact SP other_owner_hyps other_owner_stored_law (f := C) (by decide +kernel) i

theorem other_owner_denotation_boundary :
    D.leafBlock3 Q GD (.page (.name "ML")) ("Artificial Intelligence", 0) = false ∧
    D.leafBlock3 Q GDA (.page (.name "Artificial Intelligence")) ("Artificial Intelligence", 0) = true := by
  decide +kernel

theorem other_owner_reference_guarantee :
    D.References GDA ("c", 0) (.page, "Machine Learning") := by
  have he : GDA = renameBy (R.at mq).base m GD := by decide +kernel
  rw [he]
  apply D.references_merge_forward other_owner_hyps (f := C) (by decide +kernel) 0
    (t := (.page, "Machine Learning"))
  · decide +kernel
  · exact (D.referencesB_iff _ _ _).mp (by decide +kernel)

end Witness12
end CM
