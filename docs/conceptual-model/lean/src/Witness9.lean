/- Round 9 witnesses (D-20261103-CMA). Toy parser of rounds 6-8
(`QueryWitness6.D`, key = id; tokens `t:` title, `a:` alias, `l:` link,
`e:` stored `(page "...")` query; blocks split by `;`). Names with spaces are
spelled with `-` (`ML-notes` for "ML notes", `Machine-Learning`), since the
toy splits tokens at spaces. No production-parser claim. -/
import Rulings9

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace Witness9
open Params Merge7 Merge8 Rulings9
open Witnesses2 (pg blocksOf splitStr)

abbrev D : Params String := QueryWitness6.D
abbrev R : Decided.Rewriter := QueryWitness6.R
abbrev Q : QParams String String := QueryWitness6.Q
abbrev SP : QueryRename6.StoredParser String := QueryWitness6.S
abbrev JT : Merger := MergeWitness7.JT

/-! ## 1. CM-OWN-CHILD -/

/-- **W9-1 (r8b finding 1, refused).** `a → a/x` where `o` declares
`alias:: a/x`: refused for rename and for merge. -/
theorem own_child_alias_refused :
    Decided.ownChildUsed D MergeWitness8b.GaliasChild "a" "a/x" = true ∧
    Decided.rename D R MergeWitness8b.GaliasChild "a" "a/x" = none ∧
    merge D R JT MergeWitness8b.GaliasChild "a" "a/x" = none := by decide +kernel

/-- **W9-2 (real child, refused).** `Foo → Foo/bar` while the page `Foo/bar`
exists: refused for rename (round 8 accepted it, `MergeWitness8.mw_subtree_rename`)
and for merge (round 8 already refused). -/
theorem own_child_real_refused :
    Decided.ownChildUsed D MergeWitness8.GFoo "Foo" "Foo/bar" = true ∧
    Decided.rename D R MergeWitness8.GFoo "Foo" "Foo/bar" = none ∧
    merge D R JT MergeWitness8.GFoo "Foo" "Foo/bar" = none := by decide +kernel

def GUnused : Graph := [pg "Foo" "l:z", pg "c" "l:Foo"]

/-- **W9-3 (unused child, allowed).** `Foo → Foo/bar` with no `Foo/bar` anywhere. -/
theorem own_child_unused_allowed :
    Decided.ownChildUsed D GUnused "Foo" "Foo/bar" = false ∧
    Decided.renameAllowed D GUnused "Foo" "Foo/bar" = true ∧
    Decided.rename D R GUnused "Foo" "Foo/bar" =
      some [pg "Foo/bar" "l:z", pg "c" "l:Foo/bar"] := by decide +kernel

def GRefOnly : Graph := [pg "Foo" "l:z", pg "c" "l:Foo;l:Foo/bar"]

/-- **W9-4 (reference-only child: unused, confirmed D-20261105-CM11 CM-R9-USED).** `Foo/bar` is only linked: it is a page of the universe but no file
claims it and no owner declares it. The rename is allowed, and the link
`[[Foo/bar]]`, being inside `Foo`'s subtree, moves to `[[Foo/bar/bar]]`. -/
theorem own_child_reference_only :
    ((.page, "Foo/bar") : Page String) ∈ Decided.pages D GRefOnly ∧
    Decided.ownChildUsed D GRefOnly "Foo" "Foo/bar" = false ∧
    Decided.rename D R GRefOnly "Foo" "Foo/bar" =
      some [pg "Foo/bar" "l:z", pg "c" "l:Foo/bar;l:Foo/bar/bar"] := by decide +kernel

/-- **W9-5 (C9-5 instantiated).** Round 8's MB1 example (`src → tgt`, `tgt`
an alias of `owner`) through the premise-free decided form. -/
theorem rename_onto_alias_example :
    D.References (renameBy R.base MergeWitness7.mm MergeWitness8.G5)
        ((R.base.apply MergeWitness7.mm (pg "c" "l:src;l:tgt")).path, 0) (.page, D.key "tgt") ∧
    D.References (renameBy R.base MergeWitness7.mm MergeWitness8.G5)
        ((R.base.apply MergeWitness7.mm (pg "c" "l:src;l:tgt")).path, 0)
        (D.claim MergeWitness8.O2) :=
  (rename_onto_alias_decided D MergeWitness8.mw_rename_hyps (old := "src") (new := "tgt") rfl
    (by decide +kernel) (S := MergeWitness8.S2) (D := MergeWitness8.O2)
    (by decide +kernel) (by decide +kernel) (by decide +kernel) (by decide +kernel)
    (by decide +kernel) (by decide +kernel) (by decide +kernel) (by decide +kernel)).2.2.2 _
    (by decide +kernel) 0 (by decide +kernel)
    (Or.inl (D.referencesB_iff _ _ _ |>.mp (by decide +kernel)))

/-! ## 2. Merge through an alias name: ruling (a) queries, ruling (b) children -/

/-- The toy rewriter with its own query map: stored-query positions (`e:` …)
take `mq`, every other position takes `m`. At `mq = m` it is `QueryWitness6.R`. -/
def R9 : QRewriter where
  apply m mq f := { (Witnesses3.R.apply m f) with
    text := ";".intercalate ((blocksOf f.text).map (fun b =>
      " ".intercalate ((splitStr ' ' b).map (fun tok =>
        QueryWitness6.rwQueryTok mq (Witnesses3.rwTok (decide (f.dir = .pages)) m tok))))) }

/-- `MergeWitness7.JT` built from `R9` at the query map `mq`. -/
def J9 (mq : String → String) : Merger where
  apply m S T :=
    let T' := (R9.at mq).apply m T
    ⟨T.path, T'.dir, T'.format, T'.text ++ ";" ++ MergeWitness7.dropTitle ((R9.at mq).apply m S).text⟩
  origin _ _ T i :=
    let n := (blocksOf T.text).length
    if i < n then [(false, i)] else [(true, i - n)]

def SML : File := pg "ML-notes" "l:y"
def OML : File := pg "Machine-Learning" "a:ML"
def GML : Graph := [SML, OML, pg "ML-notes/x" "", pg "c" "e:ML-notes e:ML-notes/x l:ML-notes"]
def mML : String → String := nsSubst D.key D.urlHasOrigin "ML-notes" "ML"
def qML : String → String := queryMap D "ML-notes" "ML" OML
def AfterML : Graph := [pg "Machine-Learning" "a:ML;l:y", pg "ML/x" "",
  pg "c" "e:Machine-Learning e:ML/x l:ML"]
def AfterML8 : Graph := [pg "Machine-Learning" "a:ML;l:y", pg "ML/x" "",
  pg "c" "e:ML e:ML/x l:ML"]

/-- **W9-6 (rulings (a) and (b) on the example).** "Merge `ML notes` into
Machine Learning" by typing its alias `ML`: the stored `(page "ML notes")`
becomes `(page "Machine Learning")` and matches the merged page's blocks; the
child `ML notes/x` moves to `ML/x` (the typed name), and so does its stored
query; the link `[[ML notes]]` becomes `[[ML]]`, which counts for Machine
Learning. Round 8's merge (`AfterML8`) wrote `(page "ML")`, which matches
nothing (MW18). -/
theorem ml_merge :
    mergeTarget D GML "ML-notes" "ML" = some OML ∧
    merge9 D R9 J9 GML "ML-notes" "ML" = some AfterML ∧
    merge D R JT GML "ML-notes" "ML" = some AfterML8 ∧
    QueryRename6.storedAt D SP AfterML ("c", 0) =
      [.leaf (.page (.name "Machine-Learning")), .leaf (.page (.name "ML/x"))] ∧
    Decided.den D Q AfterML .block (.leaf (.page (.name "Machine-Learning"))) =
      [.block ("Machine-Learning", 0), .block ("Machine-Learning", 1)] ∧
    Decided.den D Q AfterML .block (.leaf (.page (.name "ML/x"))) = [.block ("ML/x", 0)] ∧
    Decided.den D Q AfterML8 .block (.leaf (.page (.name "ML"))) = [] ∧
    D.referencesB AfterML ("c", 0) (.page, "Machine-Learning") = true := by decide +kernel

/-- The merge's page map: the source key goes to the owner. -/
def τML (k : String) : String := if k = "ML-notes" then "Machine-Learning" else mML k

/-- **W9-7 (the Q9 hypotheses hold on the example).** -/
theorem ml_base : MergeBase D (R9.at qML).base (J9 qML) mML τML SML OML GML mML where
  lawful := Policy5.lawful _ _ Witnesses3.lawful
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
      have h1 : (D.outline SML.format SML.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline OML.format OML.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }

theorem ml_query_laws : QueryLaw9 D SP (J9 qML) mML qML SML OML ∧
    StoredOthers9 D SP (R9.at qML).base mML qML SML OML GML := by
  constructor
  · unfold QueryLaw9; decide +kernel
  · unfold StoredOthers9; decide +kernel

theorem ml_after : mergeBy (R9.at qML).base (J9 qML) mML SML OML GML = AfterML := by decide +kernel

/-- **W9-8 (Q9-8/Q9-9 instantiated).** -/
theorem ml_page_leaf :
    D.leafBlock3 Q AfterML (.page (.name (qML "ML-notes"))) ("Machine-Learning", 1) = true ∧
    D.leafBlock3 Q AfterML (.page (.name (qML "ML-notes"))) ("c", 0) = false := by
  rw [← ml_after]
  exact ⟨pageLeaf_survivor9 Q ml_base (old := "ML-notes") (new := "ML") rfl 1,
    pageLeaf_other9 Q ml_base (old := "ML-notes") (new := "ML") rfl
      (f := pg "c" "e:ML-notes e:ML-notes/x l:ML-notes") (by decide +kernel) 0⟩

/-- **W9-9 (direct merges unchanged).** Where the target claims the typed
name, `merge9` gives round 7/8's after-graph (`MergeWitness7.mw_success`). -/
theorem direct_merge_unchanged :
    merge9 D R9 J9 MergeWitness7.G1 "src" "tgt" = merge D R JT MergeWitness7.G1 "src" "tgt" ∧
    merge9 D R9 J9 MergeWitness7.G1 "src" "tgt" = some MergeWitness7.After1 := by decide +kernel

/-! ## 3. EMPTY-ALIAS -/

def GE : Graph := [pg "ML" "", OML, pg "c" "l:ML"]
def GN : Graph := [pg "ML" "body", OML, pg "c" "l:ML"]

/-- **W9-10 (the ruling).** An empty `ML` next to `Machine Learning` with
`alias:: ML`: `[[ML]]` opens Machine Learning (round 8 opened `ML`); with
content, `ML` opens itself. -/
theorem empty_alias_opens_owner :
    D.contentFree (pg "ML" "") = true ∧
    D.resolve GE "ML" = (.page, "Machine-Learning") ∧
    resolveKeyR8 D GE "ML" = (.page, "ML") ∧
    D.contentFree (pg "ML" "body") = false ∧
    D.resolve GN "ML" = (.page, "ML") := by decide +kernel

/-- **W9-11 (what the empty page otherwise is).** Still a real, listed page;
the block linking `[[ML]]` appears in the Linked References of both `ML` and
Machine Learning (one alias class), as in round 8. -/
theorem empty_alias_still_page :
    D.real GE (.page, "ML") = true ∧
    ((.page, "ML") : Page String) ∈ Decided.pages D GE ∧
    D.referencesB GE ("c", 0) (.page, "ML") = true ∧
    D.referencesB GE ("c", 0) (.page, "Machine-Learning") = true := by decide +kernel

/-- **W9-12 (OG's emptiness, not "no body").** A file holding only a property
or only a title is not content-free (OG keeps its pre-block) and wins; two
blank blocks are not content-free either (OG's dummy test admits exactly one
blank block; the lone blank block `pg "ML" ""` is W9-10's content-free file);
without a declarer an empty file opens itself. -/
theorem content_boundary :
    D.resolve [pg "ML" "p:type", OML] "ML" = (.page, "ML") ∧
    D.resolve [pg "ML" "t:ML", OML] "ML" = (.page, "ML") ∧
    D.resolve [pg "ML" ";", OML] "ML" = (.page, "ML") ∧
    D.resolve [pg "ML" "", pg "c" "l:ML"] "ML" = (.page, "ML") := by decide +kernel

/-- **W9-13 (rename of the empty alias page).** `ML → ML2`: the file moves, the
owner's `alias:: ML` and the link follow (the decided alias rewrite), and
`[[ML2]]` opens Machine Learning: the empty page keeps redirecting. -/
theorem rename_empty_alias_page :
    Decided.rename D R GE "ML" "ML2" =
      some [pg "ML2" "", pg "Machine-Learning" "a:ML2", pg "c" "l:ML2"] ∧
    D.resolve [pg "ML2" "", pg "Machine-Learning" "a:ML2", pg "c" "l:ML2"] "ML2" =
      (.page, "Machine-Learning") := by decide +kernel

/-- **W9-14 (rename of the owner).** `Machine-Learning → ML-main`: the alias
stays, `[[ML]]` now opens `ML-main`. -/
theorem rename_owner_of_empty_alias :
    Decided.rename D R GE "Machine-Learning" "ML-main" =
      some [pg "ML" "", pg "ML-main" "a:ML", pg "c" "l:ML"] ∧
    D.resolve [pg "ML" "", pg "ML-main" "a:ML", pg "c" "l:ML"] "ML" = (.page, "ML-main") := by
  decide +kernel

def GQ5 : Graph := [pg "ML-notes" "l:y", pg "ML" "", OML]

/-- **W9-15 (legacy direct merge onto an empty alias page; superseded by Rulings11.answer).** `ML.md`
exists but is empty, so `[[ML]]` opens Machine Learning; renaming `ML notes`
to `ML` was a merge into the empty file `ML` (it claims the name), not the
M-Q5 alias dialog. Round 11 supersedes that offer, retaining this low-level outcome.
The result is M-Q5(b)'s outcome: `ML` gets the content
and opens itself, still an alias of Machine Learning. -/
theorem rename_onto_empty_alias_page :
    D.resolve GQ5 "ML" = (.page, "Machine-Learning") ∧
    Decided.renameAllowed D GQ5 "ML-notes" "ML" = false ∧
    mergeTarget D GQ5 "ML-notes" "ML" = some (pg "ML" "") ∧
    merge D R JT GQ5 "ML-notes" "ML" = some [pg "ML" ";l:y", OML] ∧
    D.resolve [pg "ML" ";l:y", OML] "ML" = (.page, "ML") := by decide +kernel

/-! ## 4. CM-NAMES (2): a journal keeps its date name -/

def DayFile : File := ⟨"2026-10-26", .journals, .md, "t:Meeting"⟩
def GJ : Graph := [DayFile, pg "c" "l:2026-10-26;l:Meeting"]

/-- **W9-16.** A journal file with `title:: Meeting` is the journal page of
its date; `Meeting` names no file (here it is a reference-only page). -/
theorem journal_keeps_date :
    D.name DayFile = "2026-10-26" ∧
    D.claim DayFile = (.journal, "2026-10-26") ∧
    D.resolve GJ "2026-10-26" = (.journal, "2026-10-26") ∧
    D.real GJ (.page, "Meeting") = false ∧
    D.resolve GJ "Meeting" = (.page, "Meeting") ∧
    D.referencesB GJ ("c", 1) (.journal, "2026-10-26") = false := by decide +kernel

end Witness9
end CM
