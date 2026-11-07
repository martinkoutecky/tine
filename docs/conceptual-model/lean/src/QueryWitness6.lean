/- Finite text fixtures, not a production query grammar. No old fixture is edited. -/
import QueryRename6

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace QueryWitness6
open Params QueryRename6
open Witnesses2 (pg blocksOf toks splitStr tokVal allToks)

def D : Params String := Policy5.params Witnesses3.T Witnesses5.dateKey

/-- v:refs, w:name, e:page(name), j:page(page(name)), u:opaque given,
z:one mixed Boolean query, f:not(name), c:not(refs). -/
def queryTok (tok : String) : Option (Query String) :=
  match tok.toList with
  | x :: ':' :: rest =>
    let s := String.ofList rest
    if x = 'v' then some (.leaf (.refs s))
    else if x = 'w' then some (.leaf (.name s))
    else if x = 'e' then some (.leaf (.page (.name s)))
    else if x = 'j' then some (.leaf (.page (.page (.name s))))
    else if x = 'u' then some (.leaf (.given s))
    else if x = 'z' then some (.and (.or (.leaf (.refs s)) (.leaf (.page (.name s))))
      (.not (.leaf (.name s))))
    else if x = 'f' then some (.not (.leaf (.name s)))
    else if x = 'c' then some (.not (.leaf (.refs s)))
    else none
  | _ => none

def S : StoredParser String := ⟨fun _ b => (toks b).filterMap queryTok⟩

def Q : QParams String String where
  levels _ t := (blocksOf t).map (fun b => (toks b).countP (· == "d"))
  preBlock _ _ := false
  blockLeaf _ _ _ := true
  pageLeaf _ _ _ := true

/-- Only page-name positions are changed; u:opaque text is kept verbatim. -/
def rwQueryTok (m : String → String) (tok : String) : String :=
  match tok.toList with
  | x :: ':' :: rest =>
    if x = 'v' || x = 'w' || x = 'e' || x = 'j' || x = 'z' || x = 'f' || x = 'c' then
      String.ofList (x :: ':' :: (m (String.ofList rest)).toList)
    else tok
  | _ => tok

def R : Decided.Rewriter where
  apply m f := { (Witnesses3.R.apply m f) with
    text := ";".intercalate ((blocksOf f.text).map (fun b =>
      " ".intercalate ((splitStr ' ' b).map (fun tok =>
        rwQueryTok m (Witnesses3.rwTok (decide (f.dir = .pages)) m tok))))) }

def m : String → String := nsSubst D.key D.urlHasOrigin "old" "new"
def Before : Graph := [pg "old" "a:aka v:old w:old e:old z:old j:old u:old",
  pg "source" "l:aka;d v:old"]
def After : Graph := renameBy R.base m Before

theorem positive_success : Decided.rename D R Before "old" "new" = some After ∧
    Before ≠ After := by decide +kernel

theorem positive_faithful : FaithfulOn D S R m Before where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem positive_hyps : D.RenameHyps R.base m m Before where
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
    ns := by decide +kernel
    names := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

theorem positive_exceptionFree : D.GraphExceptionFree R.base m Before := by
  unfold GraphExceptionFree ExceptionFree; decide +kernel

theorem positive_depths : ∀ f ∈ Before,
    Q.levels (R.apply m f).format (R.apply m f).text = Q.levels f.format f.text := by
  decide +kernel

theorem positive_arguments : ∀ q ∈ storedAt D S Before ("old", 0),
    ∀ s ∈ queryArgs q, s = "old" := by decide +kernel

/-- Complete theorem instantiated for EVERY actual query in the moved block,
including the opaque given token and Boolean/nested-page queries. -/
theorem positive_image (i : Nat) (q : Query String)
    (hq : (storedAt D S Before ("old", 0))[i]? = some q) :
    ∃ q', (storedAt D S After (blockMap R.base m Before ("old", 0)))[i]? = some q' ∧
      q' = renameQuery m q ∧ ∀ a n',
      n' ∈ Decided.den D Q After a q' ↔
        ∃ n ∈ Decided.den D Q Before a q, nodeMap R.base m m Before n = n' := by
  apply stored_after_success Q S positive_success.1 positive_hyps positive_faithful
    positive_exceptionFree positive_depths (by decide +kernel) hq
  · intro s hs
    have he := positive_arguments q (List.mem_of_getElem? hq) s hs
    subst s
    exact argSafe_inPlay positive_hyps (by apply D.inPlay_iff.mpr; decide +kernel) rfl
  · intro l hl
    clear hl
    induction l with
    | refs | name => trivial
    | page l ih => exact ih
    | given => exact ⟨fun _ _ => rfl, fun _ _ => rfl⟩

/-- Model-defined-only corollary also has an inhabited, nonempty instance. -/
theorem positive_defined_image :
    ∃ q', (storedAt D S After ("new", 0))[0]? = some q' ∧
      q' = renameQuery m (.leaf (.refs "old") : Query String) ∧ ∀ a n',
      n' ∈ Decided.den D Q After a q' ↔
        ∃ n ∈ Decided.den D Q Before a (.leaf (.refs "old")),
          nodeMap R.base m m Before n = n' := by
  have h := stored_defined_image Q S positive_hyps positive_faithful positive_exceptionFree
    positive_depths (b := ("old", 0)) (i := 0) (q := .leaf (.refs "old"))
    (by decide +kernel) (by decide +kernel)
    (by
      intro s hs
      have : s = "old" := by simpa [queryArgs, Query.leaves, leafArgs] using hs
      subst s
      exact argSafe_inPlay positive_hyps (by apply D.inPlay_iff.mpr; decide +kernel) rfl)
    (by simp [Defined, Query.leaves, leafDefined])
  have he : blockMap R.base m Before ("old", 0) = ("new", 0) := by decide +kernel
  rw [he] at h
  exact h

theorem positive_readings :
    storedAt D S After ("new", 0) =
      [.leaf (.refs "new"), .leaf (.name "new"), .leaf (.page (.name "new")),
       .and (.or (.leaf (.refs "new")) (.leaf (.page (.name "new"))))
         (.not (.leaf (.name "new"))),
       .leaf (.page (.page (.name "new"))), .leaf (.given "old")] ∧
    Decided.den D Q Before .page (.leaf (.name "old")) = [.page (.page, "old")] ∧
    Decided.den D Q After .page (.leaf (.name "new")) = [.page (.page, "new")] ∧
    Decided.den D Q Before .block (.leaf (.refs "old")) =
      [.block ("old", 0), .block ("source", 0), .block ("source", 1)] ∧
    Decided.den D Q After .block (.leaf (.refs "new")) =
      [.block ("new", 0), .block ("source", 0), .block ("source", 1)] ∧
    D.occAt Before ("source", 1) = [] ∧
    D.occAt After ("source", 1) = [] := by decide +kernel

/-- No query position becomes a reference or page seed by this extension. -/
theorem query_only_no_reference :
    let G := [pg "source" "v:absent w:absent e:absent"]
    storedAt D S G ("source", 0) ≠ [] ∧ D.refNames G ("source", 0) = [] ∧
      ((.page, "absent") : Page String) ∉ Decided.pages D G := by decide +kernel

/-- The old rewriter DOES rename the page and its links; stored refs text stays old.
The before result and its mapped image are nonempty, the stale result is empty. -/
def Stale : Graph := [pg "old" "v:old", pg "source" "l:old"]
def StaleAfter : Graph := renameBy Witnesses3.R.base m Stale
theorem old_behavior_stale :
    Decided.rename D Witnesses3.R Stale "old" "new" = some StaleAfter ∧
    storedAt D S Stale ("old", 0) = [.leaf (.refs "old")] ∧
    storedAt D S StaleAfter ("new", 0) = [.leaf (.refs "old")] ∧
    (Decided.den D Q Stale .block (.leaf (.refs "old"))).map
      (nodeMap Witnesses3.R.base m m Stale) = [.block ("new", 0), .block ("source", 0)] ∧
    Decided.den D Q StaleAfter .block (.leaf (.refs "old")) = [] := by decide +kernel

def Merge : Graph := [pg "a" "l:b f:a"]
def mm : String → String := nsSubst D.key D.urlHasOrigin "a" "b"
def MergeAfter : Graph := renameBy R.base mm Merge
theorem merge_faithful : FaithfulOn D S R mm Merge where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem merge_changes_denotation :
    Decided.rename D R Merge "a" "b" = some MergeAfter ∧
    storedAt D S MergeAfter ("b", 0) = [.not (.leaf (.name "b"))] ∧
    .page (.page, "b") ∈ (Decided.den D Q Merge .page (.not (.leaf (.name "a")))).map
      (nodeMap R.base mm mm Merge) ∧
    .page (.page, "b") ∉ Decided.den D Q MergeAfter .page (.not (.leaf (.name "b"))) := by
  decide +kernel

def Cross : Graph := [pg "x" "c:x"]
def mc : String → String := nsSubst D.key D.urlHasOrigin "x" "n/y"
def CrossAfter : Graph := renameBy R.base mc Cross
theorem namespace_adds_results :
    Decided.rename D R Cross "x" "n/y" = some CrossAfter ∧
    storedAt D S CrossAfter ("n/y", 0) = [.not (.leaf (.refs "n/y"))] ∧
    .page (.page, "n") ∈ Decided.den D Q CrossAfter .page (.not (.leaf (.refs "n/y"))) ∧
    .page (.page, "n") ∉ (Decided.den D Q Cross .page (.not (.leaf (.refs "x")))).map
      (nodeMap R.base mc mc Cross) := by decide +kernel

def AliasOnly : Graph := [pg "old" "a:old/sub", pg "source" "l:new/sub w:new/sub"]
def AliasAfter : Graph := renameBy R.base m AliasOnly
theorem alias_only_image_suppressed :
    Decided.rename D R AliasOnly "old" "new" = some AliasAfter ∧
    storedAt D S AliasAfter ("source", 0) = [.leaf (.name "new/sub")] ∧
    .page (.page, "new/sub") ∈ (Decided.den D Q AliasOnly .page (.leaf (.name "new/sub"))).map
      (nodeMap R.base m m AliasOnly) ∧
    Decided.den D Q AliasAfter .page (.leaf (.name "new/sub")) = [] ∧
    D.resolve AliasAfter "new/sub" = (.page, "new") := by decide +kernel

def Absent : Graph := [pg "old" "w:new"]
def AbsentAfter : Graph := renameBy R.base m Absent
theorem absent_hyps : D.RenameHyps R.base m m Absent where
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
    ns := by decide +kernel
    names := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

theorem absent_argument_needed :
    Decided.rename D R Absent "old" "new" = some AbsentAfter ∧
    storedAt D S AbsentAfter ("new", 0) = [.leaf (.name "new")] ∧
    Decided.den D Q Absent .page (.leaf (.name "new")) = [] ∧
    Decided.den D Q AbsentAfter .page (.leaf (.name "new")) = [.page (.page, "new")] ∧
    m "new" = m "old" ∧ ("new" : String) ≠ "old" := by decide +kernel

theorem absent_not_safe : ¬ ArgSafe D m m Absent "new" := by
  intro h
  exact (by decide : ("new" : String) ≠ "old")
    (h.2 "old" (by apply D.inPlay_iff.mpr; decide +kernel) (by decide +kernel))

def Depth : Graph := [pg "old" "h", pg "source" "l:old;d v:old"]
def DepthAfter : Graph := renameBy R.base m Depth
def BadDepth : QParams String String := { Q with
  levels := fun fmt t => if (allToks t).contains "l:old" then Q.levels fmt t
    else (blocksOf t).map (fun _ => 0) }
theorem depth_hyps : D.RenameHyps R.base m m Depth where
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
    ns := by decide +kernel
    names := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

theorem depth_transport_needed :
    Decided.rename D R Depth "old" "new" = some DepthAfter ∧
    storedAt D S DepthAfter ("source", 1) = [.leaf (.refs "new")] ∧
    .block ("source", 1) ∈ Decided.den D BadDepth Depth .block (.leaf (.refs "old")) ∧
    .block ("source", 1) ∉ Decided.den D BadDepth DepthAfter .block (.leaf (.refs "new")) := by
  decide +kernel

def Given : Graph := [pg "old" "u:old"]
def GivenAfter : Graph := renameBy R.base m Given
def GlobalGiven : QParams String String := { Q with
  blockLeaf := fun _ G _ => (fileAt G "old").isSome,
  pageLeaf := fun _ G _ => (fileAt G "old").isSome }
theorem given_transport_needed :
    Decided.rename D R Given "old" "new" = some GivenAfter ∧
    storedAt D S GivenAfter ("new", 0) = [.leaf (.given "old")] ∧
    Decided.den D GlobalGiven Given .block (.leaf (.given "old")) = [.block ("old", 0)] ∧
    Decided.den D GlobalGiven GivenAfter .block (.leaf (.given "old")) = [] := by decide +kernel

theorem exceptions_and_conflicts :
    let G := [pg "old" "k:old r:old v:old"]
    let G' := renameBy R.base m G
    D.occAt G' ("new", 0) = [(.taskMarker, "old"), (.priority, "old")] ∧
    storedAt D S G' ("new", 0) = [.leaf (.refs "new")] ∧
    Decided.rename D R [pg "old" "v:old", pg "duplicate" "t:old"] "old" "new" = none ∧
    (Decided.rename D R [pg "old" "v:old", pg "u1" "t:u", pg "u2" "t:u"]
      "old" "new").isSome = true := by decide +kernel

theorem cross_faithful : FaithfulOn D S R mc Cross where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem alias_faithful : FaithfulOn D S R m AliasOnly where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem absent_faithful : FaithfulOn D S R m Absent where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem depth_faithful : FaithfulOn D S R m Depth where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem given_faithful : FaithfulOn D S R m Given where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel
  stored := by decide +kernel

theorem merge_exact_hyps : D.MergeHyps R.base mm mm Merge where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel
  }
  alias := by decide +kernel
  realInj := by decide +kernel

theorem cross_exact_hyps : D.MergeHyps R.base mc mc Cross where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel
  }
  alias := by decide +kernel
  realInj := by decide +kernel

theorem alias_exact_hyps : D.MergeHyps R.base m m AliasOnly where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel
  }
  alias := by decide +kernel
  realInj := by decide +kernel

/-- The old occurrence/alias contract and sufficient T3 hypotheses can both
hold while the stored query is stale: the missing law is exactly the new one. -/
theorem stale_hyps : D.RenameHyps Witnesses3.R.base m m Stale where
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
    ns := by decide +kernel
    names := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

theorem old_faithful : Decided.FaithfulOn D Witnesses3.R m Stale where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel

theorem old_not_query_faithful : ¬ FaithfulOn D S Witnesses3.R m Stale := by
  intro h
  have he := h.stored (pg "old" "v:old") (by decide +kernel)
  exact (by decide +kernel : ¬ (
    (D.outline (Witnesses3.R.apply m (pg "old" "v:old")).format
      (Witnesses3.R.apply m (pg "old" "v:old")).text).map
      (S.extract (Witnesses3.R.apply m (pg "old" "v:old")).format) =
    ((D.outline .md "v:old").map (S.extract .md)).map (List.map (renameQuery m)))) he

/-- Even query-only descendants follow the subtree map; opaque text does not. -/
theorem query_namespace_and_opaque :
    let G := [pg "old" "v:old/sub u:old/sub"]
    let G' := renameBy R.base m G
    Decided.rename D R G "old" "new" = some G' ∧
    storedAt D S G' ("new", 0) = [.leaf (.refs "new/sub"), .leaf (.given "old/sub")] ∧
    ((.page, "old/sub") : Page String) ∉ Decided.pages D G ∧
    ((.page, "new/sub") : Page String) ∉ Decided.pages D G' := by decide +kernel

end QueryWitness6
end CM

open Lean Elab Command in
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``CM.QueryRename6.storedAt_some,
    ``CM.QueryRename6.storedAt_rewrite,
    ``CM.QueryRename6.storedAt_blockMap,
    ``CM.QueryRename6.argSafe_inPlay,
    ``CM.QueryRename6.aliasClass_arg,
    ``CM.QueryRename6.ancestors_blocks,
    ``CM.QueryRename6.pageOfBlock_rewrite,
    ``CM.QueryRename6.ancestors_rewrite,
    ``CM.QueryRename6.refKeys_at_rewrite,
    ``CM.QueryRename6.pathKeys_inPlay,
    ``CM.QueryRename6.pathKeys_rewrite,
    ``CM.QueryRename6.defined_givenSafe,
    ``CM.QueryRename6.leafPage_transport,
    ``CM.QueryRename6.leafBlock_transport,
    ``CM.QueryRename6.eval_transport,
    ``CM.QueryRename6.univ_image,
    ``CM.QueryRename6.den_image,
    ``CM.QueryRename6.stored_den_image,
    ``CM.QueryRename6.stored_defined_image,
    ``CM.QueryRename6.given_fixed,
    ``CM.QueryRename6.leaf_args_renamed,
    ``CM.QueryRename6.stored_after_success,
    ``CM.QueryWitness6.positive_success,
    ``CM.QueryWitness6.positive_faithful,
    ``CM.QueryWitness6.positive_hyps,
    ``CM.QueryWitness6.positive_exceptionFree,
    ``CM.QueryWitness6.positive_depths,
    ``CM.QueryWitness6.positive_arguments,
    ``CM.QueryWitness6.positive_image,
    ``CM.QueryWitness6.positive_defined_image,
    ``CM.QueryWitness6.positive_readings,
    ``CM.QueryWitness6.query_only_no_reference,
    ``CM.QueryWitness6.old_behavior_stale,
    ``CM.QueryWitness6.merge_faithful,
    ``CM.QueryWitness6.merge_changes_denotation,
    ``CM.QueryWitness6.namespace_adds_results,
    ``CM.QueryWitness6.alias_only_image_suppressed,
    ``CM.QueryWitness6.absent_hyps,
    ``CM.QueryWitness6.absent_argument_needed,
    ``CM.QueryWitness6.absent_not_safe,
    ``CM.QueryWitness6.depth_hyps,
    ``CM.QueryWitness6.depth_transport_needed,
    ``CM.QueryWitness6.given_transport_needed,
    ``CM.QueryWitness6.exceptions_and_conflicts,
    ``CM.QueryWitness6.cross_faithful,
    ``CM.QueryWitness6.alias_faithful,
    ``CM.QueryWitness6.absent_faithful,
    ``CM.QueryWitness6.depth_faithful,
    ``CM.QueryWitness6.given_faithful,
    ``CM.QueryWitness6.merge_exact_hyps,
    ``CM.QueryWitness6.cross_exact_hyps,
    ``CM.QueryWitness6.alias_exact_hyps,
    ``CM.QueryWitness6.stale_hyps,
    ``CM.QueryWitness6.old_faithful,
    ``CM.QueryWitness6.old_not_query_faithful,
    ``CM.QueryWitness6.query_namespace_and_opaque]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "missing checked constant {n}"
    if info.isUnsafe then throwError "unsafe dependency {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "nonstandard axiom {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some v := info.value? (allowOpaque := true) then
      pending := v.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
  logInfo m!"ROUND 6 STORED QUERY + WITNESS CONE: {seen.size} constants; axioms {axs.toList}"
