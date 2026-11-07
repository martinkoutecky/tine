/- Round 7 merge witnesses. D is the round-6 stored-query instance (toy
parser, key = id) and R its rewriter (links, aliases, titles, property keys,
stored-query page arguments). JT is a toy survivor builder in the shape of
Tine's `merged_survivor`: the rewritten target's blocks, then every rewritten
source block. -/
import Merge7

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace MergeWitness7
open Params Merge7
open Witnesses2 (pg blocksOf toks splitStr tokVal)

abbrev D : Params String := QueryWitness6.D
abbrev R : Decided.Rewriter := QueryWitness6.R

/-- The toy has no page-header region: a `t:` token anywhere would rebind the
page name, so the survivor drops the source's `t:` tokens (Tine moves the
source's `title::` line into an ordinary block, where it no longer names the
page). Alias tokens anywhere are alias declarations, so the union is automatic. -/
def dropTitle (t : String) : String :=
  ";".intercalate ((blocksOf t).map (fun b =>
    " ".intercalate ((splitStr ' ' b).filter (fun tok => (tokVal 't' tok).isNone))))

def JT : Merger where
  apply m S T :=
    let T' := R.apply m T
    ⟨T.path, T'.dir, T'.format, T'.text ++ ";" ++ dropTitle (R.apply m S).text⟩
  origin _ _ T i :=
    let n := (blocksOf T.text).length
    if i < n then [(false, i)] else [(true, i - n)]

def G1 : Graph := [pg "src" "a:aka l:x;l:tgt v:tgt", pg "tgt" "a:other;l:src",
  pg "c" "l:src;l:aka;v:src;c:tgt", pg "src/kid" "l:tgt", pg "x" ""]
def S1 : File := pg "src" "a:aka l:x;l:tgt v:tgt"
def T1 : File := pg "tgt" "a:other;l:src"
def mm : String → String := nsSubst D.key D.urlHasOrigin "src" "tgt"
def After1 : Graph := [pg "tgt" "a:other;l:tgt;a:aka l:x;l:tgt v:tgt",
  pg "c" "l:tgt;l:aka;v:tgt;c:tgt", pg "tgt/kid" "l:tgt", pg "x" ""]

/-- **MW1 (accepted merge, after-graph).** Rename refuses (`tgt` is a real page
outside the subtree); merge accepts. The survivor keeps `tgt`'s path, its own
blocks (its link to `src` now reads `tgt`) and appends every source block; the
namespace child moves to `tgt/kid`; the referrer's link, alias use and stored
query follow. -/
theorem mw_success : Decided.renameAllowed D G1 "src" "tgt" = false ∧
    mergeSource D G1 "src" = some S1 ∧ mergeTarget D G1 "src" "tgt" = some T1 ∧
    merge D R JT G1 "src" "tgt" = some After1 ∧
    mergeBy R.base JT mm S1 T1 G1 = After1 := by decide +kernel

theorem mw_hyps : MergeOpHyps D R.base JT mm mm S1 T1 G1 where
  lawful := Policy5.lawful _ _ Witnesses3.lawful
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  src := by decide +kernel
  dst := by decide +kernel
  ne := by decide +kernel
  srcPage := by decide +kernel
  dstPage := by decide +kernel
  dstAlias := by decide +kernel
  srcKey := by decide +kernel
  dstKey := by decide +kernel
  srcUniq := by decide +kernel
  dstUniq := by decide +kernel
  aliasPage := by decide +kernel
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
      have h2 : (D.outline S1.format S1.text).length = 2 := by decide +kernel
      rw [h2] at hj
      match j, hj with
      | 0, _ => decide +kernel
      | 1, _ => decide +kernel
    coverD := by
      intro j hj
      have h2 : (D.outline T1.format T1.text).length = 2 := by decide +kernel
      rw [h2] at hj
      match j, hj with
      | 0, _ => decide +kernel
      | 1, _ => decide +kernel
    namesL := by decide +kernel }

/-- **MW2 (pages and alias classes).** `src` and `src/kid` leave the page
universe, `tgt/kid` enters; `src`'s alias `aka` joins `tgt`'s class. -/
theorem mw_pages : Decided.pages D G1 =
      [(.page, "src"), (.page, "tgt"), (.page, "c"), (.page, "src/kid"), (.page, "x")] ∧
    Decided.pages D After1 = [(.page, "tgt"), (.page, "c"), (.page, "tgt/kid"), (.page, "x")] ∧
    D.aliasEdges G1 = [("src", "aka"), ("tgt", "other")] ∧
    D.aliasEdges After1 = [("tgt", "other"), ("tgt", "aka")] := by decide +kernel

theorem mw_no_block_lost : ∀ b ∈ D.blocks G1,
    ∃ b' ∈ D.blocks (mergeBy R.base JT mm S1 T1 G1), Img R.base JT mm S1 T1 G1 b b' :=
  fun _ hb => no_block_lost mw_hyps.toMergeBase hb

/-- **MW3 (Linked References of the merged page).** Before: `src` is referenced
by `tgt`'s block 1, and by `c`'s blocks 0 (link) and 1 (alias `aka`); `tgt` by
`src`'s block 1 and `src/kid`. After: `tgt` is referenced by exactly the images
of both lists. `tgt`'s own block 1 now references its own page. -/
theorem mw_linked :
    (D.blocks G1).filter (fun b => D.referencesB G1 b (.page, "src")) =
      [("tgt", 1), ("c", 0), ("c", 1)] ∧
    (D.blocks G1).filter (fun b => D.referencesB G1 b (.page, "tgt")) =
      [("src", 1), ("src/kid", 0)] ∧
    (D.blocks After1).filter (fun b => D.referencesB After1 b (.page, "tgt")) =
      [("tgt", 1), ("tgt", 3), ("c", 0), ("c", 1), ("tgt/kid", 0)] := by decide +kernel

theorem mw_refs_to_source : ∃ b' ∈ D.blocks (mergeBy R.base JT mm S1 T1 G1),
    Img R.base JT mm S1 T1 G1 ("c", 1) b' ∧
      D.References (mergeBy R.base JT mm S1 T1 G1) b' (D.claim T1) :=
  references_to_source mw_hyps.toMergeBase (by decide +kernel) (by unfold NsForward; decide +kernel)
    (D.referencesB_iff _ _ _ |>.mp (by decide +kernel))

theorem mw_self_reference : D.References G1 ("tgt", 1) (.page, "src") ∧
    ¬ D.References G1 ("tgt", 1) (.page, "tgt") ∧
    D.References After1 ("tgt", 1) (.page, "tgt") :=
  ⟨D.referencesB_iff _ _ _ |>.mp (by decide +kernel),
   fun h => absurd ((D.referencesB_iff _ _ _).mpr h) (by decide +kernel),
   D.referencesB_iff _ _ _ |>.mp (by decide +kernel)⟩

/-- **MW4 (stored queries: the `not` boundary).** The referrer's stored `refs src` now
reads `refs tgt` and returns the merged page's whole Linked References; a
`not refs tgt` query whose text the merge leaves alone loses the blocks that
referenced only `src` (`c` 0 and 1): no image transport of denotations holds
for merges, even for unchanged query text. -/
theorem mw_queries :
    QueryRename6.storedAt D QueryWitness6.S G1 ("c", 2) = [.leaf (.refs "src")] ∧
    QueryRename6.storedAt D QueryWitness6.S After1 ("c", 2) = [.leaf (.refs "tgt")] ∧
    .block ("c", 0) ∈ Decided.den D QueryWitness6.Q G1 .block (.not (.leaf (.refs "tgt"))) ∧
    .block ("c", 0) ∉ Decided.den D QueryWitness6.Q After1 .block (.not (.leaf (.refs "tgt"))) ∧
    Decided.den D QueryWitness6.Q After1 .block (.leaf (.refs "tgt")) =
      [.block ("tgt", 0), .block ("tgt", 1), .block ("tgt", 2), .block ("tgt", 3),
       .block ("c", 0), .block ("c", 1), .block ("tgt/kid", 0)] := by decide +kernel

/-- **MW5 (merge into an alias owner: outside the theorems).** `tgt` is only an
alias of `owner`; Tine merges into `owner`. References are rewritten to `tgt`,
which resolves to `owner` through the alias. The theorems' `srcKey` fails
(σ sends `src` to `tgt`, not to `owner`), so this case is witnessed only.
Round 8 (M-Q5a): covered by the theorems through a page-level key map
`τ` (`Merge8.references_to_source_via` etc.; instance
`MergeWitness8.mw_alias_hyps`). The last conjunct stays true. -/
def G2 : Graph := [pg "src" "l:y", pg "owner" "a:tgt", pg "c" "l:src"]
def G2After : Graph := [pg "owner" "a:tgt;l:y", pg "c" "l:tgt"]
theorem mw_alias_owner : mergeTarget D G2 "src" "tgt" = some (pg "owner" "a:tgt") ∧
    merge D R JT G2 "src" "tgt" = some G2After ∧
    (D.blocks G2After).filter (fun b => D.referencesB G2After b (.page, "owner")) = [("c", 0)] ∧
    Decided.pages D G2After = [(.page, "owner"), (.page, "c"), (.page, "y")] ∧
    nsSubst D.key D.urlHasOrigin "src" "tgt" "src" ≠ "owner" := by decide +kernel

/-- **MW6 (refusals).** A same-key target (that is a rename); a target of
another format; a namespace child whose destination is taken; two files
claiming the target name. (Round 7b: the model no longer refuses a journal
survivor or a survivor inside the source's namespace; Tine does not refuse
them either. Those cases are boundary witnesses in `MergeWitness7b`.
Round 8: Martin refused the in-namespace survivor (M-Q6,
`MergeWitness8.mw_subtree_refused`) and allowed the journal survivor (M-Q7,
`MergeWitness8.mw_journal_hyps`).) -/
def Gsub : Graph := [pg "a" "l:z", pg "a/b" ""]
def Gcase : Graph := [pg "Foo" ""]
def Gfmt : Graph := [pg "src" "", ⟨"tgt", .pages, .org, ""⟩]
def Gkid : Graph := [pg "src" "", pg "tgt" "", pg "src/k" "", pg "tgt/k" ""]
def Gjr : Graph := [pg "src" "", ⟨"tgt", .journals, .md, ""⟩]
def Gtwo : Graph := [pg "src" "", pg "tgt" "", pg "t2" "t:tgt"]
theorem mw_refusals : mergeTarget D Gcase "Foo" "Foo" = none ∧
    mergeAllowed D Gfmt "src" "tgt" = false ∧
    mergeAllowed D Gkid "src" "tgt" = false ∧
    mergeAllowed D Gtwo "src" "tgt" = false ∧ mergeTarget D Gtwo "src" "tgt" = none := by
  decide +kernel

end MergeWitness7
end CM
