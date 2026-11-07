/- Round 11 executable witnesses. Literal names include spaces: tokens use |
and blocks use ;. This is a small fixture parser, not a production parser or
a file-name decoder. Every theorem is kernel-evaluated. -/
import Rulings11

set_option maxRecDepth 8192
set_option maxHeartbeats 2400000
namespace CM
namespace Witness11
open Params Merge7 Rulings9 Ref10 Rulings11
open Witnesses2 (pg blocksOf splitStr tokVal)

def toks (s : String) : List String := (splitStr '|' s).filter (fun t => !t.isEmpty)
def allToks (s : String) : List String := (blocksOf s).flatMap toks
def D : Params String := { QueryWitness6.D with
  title := fun _ t => ((allToks t).filterMap (tokVal 't')).head?
  aliasDecl := fun _ t => (allToks t).filterMap (tokVal 'a')
  occ := fun _ b => (toks b).filterMap Witnesses3.occTok }

def R : QRewriter where
  apply m mq f := { f with
    path := if f.dir = .pages && (D.title f.format f.text).isNone then m f.path else f.path
    text := ";".intercalate ((blocksOf f.text).map (fun b =>
      "|".intercalate ((splitStr '|' b).map (fun tok =>
        QueryWitness6.rwQueryTok mq (Witnesses3.rwTok (decide (f.dir = .pages)) m tok))))) }

def J (mq : String → String) : Merger where
  apply m S T :=
    let T' := (R.at mq).apply m T
    let S' := (R.at mq).apply m S
    { T' with path := T.path, text := T'.text ++ ";" ++ S'.text }
  origin _ _ T i := if i < (blocksOf T.text).length
    then [(false, i)] else [(true, i - (blocksOf T.text).length)]

def SP : QueryRename6.StoredParser String := ⟨fun _ b => (toks b).filterMap QueryWitness6.queryTok⟩
abbrev Q := QueryWitness6.Q
def lr (G : Graph) (s : String) : List BlockId :=
  (D.blocks G).filter (fun b => D.referencesB G b (.page, s))

/-! ## 1. No empty prefix, titles and supplied legacy stems -/

def GT : Graph := [pg "title-file" "t:/n/y", pg "links" "l:/n/y"]
def DS : Params String := { D with stem := fun f =>
  if f.path == "pages/%2Fn%2Fy.md" then "/n/y" else f.path }
def GS : Graph := [pg "pages/%2Fn%2Fy.md" "body"]

theorem leading_title :
    D.fileNsNames (pg "title-file" "t:/n/y") = ["/n"] ∧
    Decided.pages D GT = [(.page, "/n/y"), (.page, "links"), (.page, "/n"), (.page, "/n")] ∧
    lr GT "" = [] ∧ lr GT "/n" = [("links", 0)] ∧
    (lr GT "/n").length = 1 ∧
    treeParent D id GT (.page, "/n/y") = some (.page, "/n") ∧
    treeParent D id GT (.page, "/n") = none ∧
    nsMatch D.key D.urlHasOrigin "" "/n/y" = none ∧
    nsSubst D.key D.urlHasOrigin "" "q" "/n/y" = "/n/y" := by decide +kernel

/-- The stem is an explicit supplied extractor result; this proves no decoder. -/
theorem leading_legacy_stem :
    DS.name (pg "pages/%2Fn%2Fy.md" "body") = "/n/y" ∧
    DS.fileNsNames (pg "pages/%2Fn%2Fy.md" "body") = ["/n"] ∧
    Decided.pages DS GS = [(.page, "/n/y"), (.page, "/n")] ∧
    tree DS id GS = [((.page, "/n/y"), []), ((.page, "/n"), [(.page, "/n/y")])] := by
  decide +kernel

theorem leading_rename :
    answer D R J GT "/n" "z" .rename =
      some [pg "title-file" "t:z/y", pg "links" "l:z/y"] ∧
    Decided.pages D [pg "title-file" "t:z/y", pg "links" "l:z/y"] =
      [(.page, "z/y"), (.page, "links"), (.page, "z"), (.page, "z")] ∧
    (D.movedFiles GT "") = [] := by decide +kernel

/-! ## 2. Empty target: same dialog, two complete file graphs -/

def S : File := pg "ML notes" "l:y"
def E : File := pg "ML" ""
def O : File := pg "Machine Learning" "a:ML"
def C : File := pg "c" "l:ML notes|l:ML|e:ML notes|e:ML notes/x"
def GE : Graph := [S, E, O, pg "ML notes/x" "body", C]
def GM : Graph := [E, pg "Machine Learning" "a:ML;l:y", pg "ML/x" "body",
  pg "c" "l:ML|l:ML|e:Machine Learning|e:ML/x"]
def GP : Graph := [pg "ML" ";l:y", O, pg "ML/x" "body",
  pg "c" "l:ML|l:ML|e:ML|e:ML/x"]

theorem empty_target_offers :
    dialogOwner D GE "ML notes" "ML" = some O ∧
    sourceKind D GE "ML notes" = .realPage ∧
    offers D GE "ML notes" "ML" = [.mergeOwner, .ownPage, .cancel] ∧
    answer D R J GE "ML notes" "ML" .rename = none ∧
    answer D R J GE "ML notes" "ML" .cancel = some GE := by decide +kernel

theorem empty_target_merge_files :
    answer D R J GE "ML notes" "ML" .mergeOwner = some GM ∧
    (GM.map File.path) = ["ML", "Machine Learning", "ML/x", "c"] ∧
    E ∈ GM ∧ S ∉ GM ∧
    D.contentFree E = true ∧ D.resolve GM "ML" = (.page, "Machine Learning") ∧
    D.aliasEdges GM = [("Machine Learning", "ML")] := by decide +kernel

theorem empty_target_own_files :
    answer D R J GE "ML notes" "ML" .ownPage = some GP ∧
    (GP.map File.path) = ["ML", "Machine Learning", "ML/x", "c"] ∧
    S ∉ GP ∧ E ∉ GP ∧ D.contentFree (pg "ML" ";l:y") = false ∧
    D.resolve GP "ML" = (.page, "ML") ∧
    D.aliasEdges GP = [("Machine Learning", "ML")] := by decide +kernel

theorem empty_target_references :
    lr GE "ML notes" = [("c", 0)] ∧
    lr GM "Machine Learning" = [("c", 0)] ∧ lr GP "Machine Learning" = [("c", 0)] ∧
    lr GM "ML" = [("c", 0)] ∧ lr GP "ML" = [("c", 0)] ∧
    (lr GM "Machine Learning").length = (lr GP "Machine Learning").length ∧
    (lr GM "ML notes") = [] ∧ (lr GP "ML notes") = [] := by decide +kernel

theorem empty_target_queries_children :
    QueryRename6.storedAt D SP GM ("c", 0) =
      [.leaf (.page (.name "Machine Learning")), .leaf (.page (.name "ML/x"))] ∧
    QueryRename6.storedAt D SP GP ("c", 0) =
      [.leaf (.page (.name "ML")), .leaf (.page (.name "ML/x"))] ∧
    Decided.den D Q GM .block (.leaf (.page (.name "Machine Learning"))) =
      [.block ("Machine Learning", 0), .block ("Machine Learning", 1)] ∧
    Decided.den D Q GP .block (.leaf (.page (.name "ML"))) =
      [.block ("ML", 0), .block ("ML", 1)] ∧
    children D id GM (.page, "ML") = [(.page, "ML/x")] ∧
    children D id GP (.page, "ML") = [(.page, "ML/x")] := by decide +kernel

/-- No target file: both answers still work for a real source. -/
def GA : Graph := [S, O, C]
theorem absent_target_files :
    offers D GA "ML notes" "ML" = [.mergeOwner, .ownPage, .cancel] ∧
    answer D R J GA "ML notes" "ML" .mergeOwner =
      some [pg "Machine Learning" "a:ML;l:y",
        pg "c" "l:ML|l:ML|e:Machine Learning|e:ML/x"] ∧
    answer D R J GA "ML notes" "ML" .ownPage =
      some [pg "ML" "l:y", O, pg "c" "l:ML|l:ML|e:ML|e:ML/x"] := by decide +kernel

/-! ## 3. File-less and empty-alias-file sources -/

def GB : Graph := [O, pg "c" "l:B|e:B"]
def GBM : Graph := [O, pg "c" "l:ML|e:Machine Learning"]
theorem fileless_offers_and_answer :
    sourceKind D GB "B" = .fileless ∧
    offers D GB "B" "ML" = [.mergeOwner, .cancel] ∧
    answer D R J GB "B" "ML" .ownPage = none ∧
    answer D R J GB "B" "ML" .rename = none ∧
    answer D R J GB "B" "ML" .mergeOwner = some GBM ∧
    GBM.map File.path = GB.map File.path ∧
    D.resolve GBM "ML" = (.page, "Machine Learning") ∧
    QueryRename6.storedAt D SP GBM ("c", 0) = [.leaf (.page (.name "Machine Learning"))] ∧
    lr GBM "Machine Learning" = [("c", 0)] ∧ lr GBM "B" = [] := by decide +kernel

theorem fileless_empty_target :
    offers D (E :: GB) "B" "ML" = [.mergeOwner, .cancel] ∧
    answer D R J (E :: GB) "B" "ML" .mergeOwner = some (E :: GBM) ∧
    answer D R J (E :: GB) "B" "ML" .ownPage = none := by decide +kernel

def GEmptySource : Graph := [pg "ML notes" "", pg "source owner" "a:ML notes", O, C]
theorem empty_alias_source :
    sourceKind D GEmptySource "ML notes" = .emptyAliasFile ∧
    offers D GEmptySource "ML notes" "ML" = [.mergeOwner, .ownPage, .cancel] ∧
    answer D R J GEmptySource "ML notes" "ML" .ownPage =
      some [pg "ML" "", pg "source owner" "a:ML", O,
        pg "c" "l:ML|l:ML|e:ML|e:ML/x"] ∧
    D.resolve [pg "ML" "", pg "source owner" "a:ML", O] "ML" = (.page, "Machine Learning") := by
  decide +kernel

/-- A solid ML file gets an ordinary merge, not an alias dialog. -/
theorem solid_target_no_dialog :
    dialogOwner D [S, pg "ML" "body", O] "ML notes" "ML" = none ∧
    offers D [S, pg "ML" "body", O] "ML notes" "ML" = [.mergeOwner, .cancel] ∧
    offers D [O, C] "ML" "new" = [.rename, .cancel] := by decide +kernel

/-! ## 4. Link-only child is not used: no statement change to W9-4 -/
theorem linked_child_confirmed :
    Decided.ownChildUsed D [pg "Foo" "body", pg "c" "l:Foo/bar"] "Foo" "Foo/bar" = false ∧
    offers D [pg "Foo" "body", pg "c" "l:Foo/bar"] "Foo" "Foo/bar" = [.rename, .cancel] ∧
    answer D R J [pg "Foo" "body", pg "c" "l:Foo/bar"] "Foo" "Foo/bar" .rename =
      some [pg "Foo/bar" "body", pg "c" "l:Foo/bar/bar"] ∧
    offers D [pg "Foo" "body", pg "o" "a:Foo/bar"] "Foo" "Foo/bar" = [.cancel] := by
  decide +kernel

/-! ## 5. Exported tree: alias spellings have no node or children row -/
def GTree : Graph := [pg "old" "body", pg "owner" "a:old/sub", pg "c" "l:old/sub",
  pg "old/real" "body"]
theorem alias_tree :
    treeNodes D GTree = [(.page, "old"), (.page, "owner"), (.page, "c"), (.page, "old/real")] ∧
    tree D id GTree = [((.page, "old"), [(.page, "old/real")]), ((.page, "owner"), []),
      ((.page, "c"), []), ((.page, "old/real"), [])] ∧
    children D id GTree (.page, "old") = [(.page, "old/real")] ∧
    treeParent D id GTree (.page, "old/sub") = none ∧
    D.resolve GTree "old/sub" = (.page, "owner") ∧
    namespaceTree D id GTree = [((.page, "old"), [(.page, "old/real")]),
      ((.page, "old/real"), [])] := by decide +kernel

theorem real_alias_tree :
    (treeNodes D (pg "old/sub" "body" :: GTree)).contains (.page, "old/sub") = true ∧
    children D id (pg "old/sub" "body" :: GTree) (.page, "old") =
      [(.page, "old/sub"), (.page, "old/real")] := by decide +kernel


/-! ## 6. Existing semantic guarantees instantiated on the NEW empty-target cases -/
def m : String → String := nsSubst D.key D.urlHasOrigin "ML notes" "ML"
def tau (s : String) : String := if s = "ML notes" then "Machine Learning" else m s
def qm : String → String := queryMap D "ML notes" "ML" O
def qp : String → String := queryMap D "ML notes" "ML" E

/-- The old MergeBase hypotheses admit an empty alias target, without a new exclusion. -/
theorem empty_merge_base :
    MergeBase D (R.at qm).base (J qm) m tau S O GE m where
  lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  src := by decide +kernel
  dst := by decide +kernel
  ne := by decide +kernel
  srcPage := by decide +kernel
  srcKey := by decide +kernel
  dstKey := by decide +kernel
  srcUniq := by decide +kernel
  dstUniq := by decide +kernel
  aliasPage := by decide +kernel
  dstAlias := by decide +kernel
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
  merger := {
    path := by decide +kernel
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    aliasL := by decide +kernel
    nonempty := by decide +kernel
    valid := by decide +kernel
    occSubL := by decide +kernel
    coverS := by
      intro j hj
      have hn : (D.outline S.format S.text).length = 1 := by decide +kernel
      rw [hn] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have hn : (D.outline O.format O.text).length = 1 := by decide +kernel
      rw [hn] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }

theorem empty_merge_stored_laws :
    QueryLaw9 D SP (J qm) m qm S O ∧
    StoredOthers9 D SP (R.at qm).base m qm S O GE := by
  constructor
  · unfold QueryLaw9; decide +kernel
  · unfold StoredOthers9; decide +kernel

/-- Apply Q9-8 itself, rather than replacing its guarantee with an unrelated witness. -/
theorem empty_merge_page_leaf :
    D.leafBlock3 Q (mergeBy (R.at qm).base (J qm) m S O GE)
      (.page (.name (queryMap D "ML notes" "ML" O "ML notes"))) (O.path, 1) = true :=
  pageLeaf_survivor9 (P := D) Q empty_merge_base (by decide +kernel) 1

/-- The old MergeBase hypotheses admit an empty alias target, without a new exclusion. -/
theorem empty_own_base :
    MergeBase D (R.at qp).base (J qp) m m S E GE m where
  lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  src := by decide +kernel
  dst := by decide +kernel
  ne := by decide +kernel
  srcPage := by decide +kernel
  srcKey := by decide +kernel
  dstKey := by decide +kernel
  srcUniq := by decide +kernel
  dstUniq := by decide +kernel
  aliasPage := by decide +kernel
  dstAlias := by decide +kernel
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
  merger := {
    path := by decide +kernel
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    aliasL := by decide +kernel
    nonempty := by decide +kernel
    valid := by decide +kernel
    occSubL := by decide +kernel
    coverS := by
      intro j hj
      have hn : (D.outline S.format S.text).length = 1 := by decide +kernel
      rw [hn] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have hn : (D.outline E.format E.text).length = 1 := by decide +kernel
      rw [hn] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }

theorem empty_own_stored_laws :
    QueryLaw9 D SP (J qp) m qp S E ∧
    StoredOthers9 D SP (R.at qp).base m qp S E GE := by
  constructor
  · unfold QueryLaw9; decide +kernel
  · unfold StoredOthers9; decide +kernel

/-- Apply Q9-8 itself, rather than replacing its guarantee with an unrelated witness. -/
theorem empty_own_page_leaf :
    D.leafBlock3 Q (mergeBy (R.at qp).base (J qp) m S E GE)
      (.page (.name (queryMap D "ML notes" "ML" E "ML notes"))) (E.path, 1) = true :=
  pageLeaf_survivor9 (P := D) Q empty_own_base (by decide +kernel) 1

theorem empty_merge_via_alias : Merge8.ViaAlias D m tau S O GE where
  declares := by decide +kernel
  fixed := by decide +kernel
  agree := by
    intro k hk
    change k ≠ "ML notes" at hk
    simp [tau, hk]

/-- MA2 instantiated with the empty ML file still present: source references reach owner. -/
theorem empty_merge_reference_guarantee :
    ∃ b' ∈ D.blocks (mergeBy (R.at qm).base (J qm) m S O GE),
      Merge7.Img (R.at qm).base (J qm) m S O GE ("c", 0) b' ∧
      D.References (mergeBy (R.at qm).base (J qm) m S O GE) b' (D.claim O) :=
  Merge8.references_to_source_via empty_merge_base empty_merge_via_alias
    (by decide +kernel) (by unfold NsForward; decide +kernel)
    ((D.referencesB_iff _ _ _).mp (by decide +kernel))

/-- Ambiguous alias owners cannot unlock the forbidden file-less rewrite. -/
theorem fileless_multi_owner :
    offers D [O, pg "other owner" "a:ML", pg "c" "l:B"] "B" "ML" = [.cancel] ∧
    answer D R J [O, pg "other owner" "a:ML", pg "c" "l:B"] "B" "ML" .rename = none ∧
    offers D [E, O, pg "other owner" "a:ML", pg "c" "l:B"] "B" "ML" = [.cancel] ∧
    offers D [S, E, O, pg "other owner" "a:ML"] "ML notes" "ML" = [.cancel] := by
  decide +kernel

end Witness11
end CM
