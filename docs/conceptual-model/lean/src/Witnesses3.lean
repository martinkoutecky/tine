/- Kernel-checked round-3 witnesses. T is the toy parser extended with task
and priority occurrences. R rewrites property keys and aliases, keeps task and
priority spellings, and never rebinds a journal's name or path. -/
import Isolation
import TransportLinks
import Witnesses2

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace Witnesses3
open Params
open Witnesses2 (pg fav splitStr blocksOf toks allToks tokVal)

def occTok (tok : String) : Option (OccKind × String) :=
  match tokVal 'k' tok with
  | some s => some (.taskMarker, s)
  | none => match tokVal 'r' tok with
    | some s => some (.priority, s)
    | none => Witnesses2.occTok tok

def T : Params String := { Witnesses2.T with occ := fun _ t => (toks t).filterMap occTok }
theorem lawful : T.Lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
theorem keyLaws : KeyLaws T.key id := ⟨fun _ => rfl⟩

def TJ : Params String := { T with aliasKinds := fun _ => true }
def JournalAlias : Graph := [⟨"j", .journals, .md, "a:x"⟩]
theorem open_alias_policy : T.aliasEdges JournalAlias = [] ∧
    TJ.aliasEdges JournalAlias = [("j", "x")] ∧
    TJ.resolve JournalAlias "x" = (.journal, "j") ∧
    ((.page, "x") : Page String) ∉ Decided.pages TJ JournalAlias := by decide +kernel

def rwTok (moveName : Bool) (m : String → String) (tok : String) : String :=
  match tok.toList with
  | x :: ':' :: rest =>
    if (moveName && x = 't') || x = 'a' || x = 'l' || x = 'g' || x = 'p' then
      String.ofList (x :: ':' :: (m (String.ofList rest)).toList)
    else tok
  | _ => tok

def R : Decided.Rewriter where
  apply m f := { f with
    path := if f.dir = .pages && (T.title f.format f.text).isNone then m f.path else f.path
    text := ";".intercalate ((blocksOf f.text).map (fun b =>
      " ".intercalate ((splitStr ' ' b).map (rwTok (decide (f.dir = .pages)) m)))) }

def Q : QParams String Witnesses2.GL := Witnesses2.Q

def Good : Graph := [pg "f-old" "h t:old;l:c", pg "old/sub" "l:p", pg "p" "h a:old",
  pg "c" "l:old/sub p:old;d g:old;d d l:old/ref"]
def m : String → String := nsSubst T.key T.urlHasOrigin "old" "new"
def Good' : Graph := T.rename R.base Good "old" "new"

theorem good_clean : T.conflictFreeB Good = true := by decide +kernel
theorem good_allowed : Decided.renameAllowed T Good "old" "new" = true := by decide +kernel
theorem good_moves : Decided.rename T R Good "old" "new" = some Good' ∧ Good ≠ Good' := by
  decide +kernel

theorem good_faithful : Decided.FaithfulOn T R m Good where
  dir := by decide +kernel
  format := by decide +kernel
  name := by decide +kernel
  path := by decide +kernel
  occ := by decide +kernel
  alias := by decide +kernel

theorem good_hyps : T.RenameHyps R.base m m Good where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := Decided.faithful_rwLaws T lawful (T.conflictFreeB_iff.mp good_clean)
    good_faithful (by decide +kernel) (by decide +kernel)
  keys := {
    alias := by decide +kernel
    names := by decide +kernel, ns := by decide +kernel
    inj := by decide +kernel
    refKind := by decide +kernel }

theorem good_exceptionFree : T.GraphExceptionFree R.base m Good := by
  unfold GraphExceptionFree ExceptionFree; decide +kernel

theorem good_features :
    T.aliasEdges Good = [("p", "old")] ∧ T.aliasEdges Good' = [("p", "new")] ∧
    T.occAt Good' ("c", 0) = [(.link, "new/sub"), (.propertyKey, "new")] ∧
    ((.page, "new/ref") : Page String) ∈ Decided.pages T Good' ∧
    ((.page, "old") : Page String) ∉ Decided.pages T Good' := by decide +kernel

theorem good_refs : T.References Good ("c", 1) (.page, "p") ∧
    T.References Good' ("c", 1) (.page, "p") := by
  have hr : T.References Good ("c", 1) (.page, "p") :=
    T.referencesB_iff _ _ _ |>.mp (by decide +kernel)
  exact ⟨hr, (references_rename good_hyps (by decide +kernel)
    (good_exceptionFree _ (by decide +kernel)) (by
      apply T.inPlay_iff.mpr; decide +kernel)).mp hr⟩

theorem good_pages :
    (∀ p ∈ Decided.pages T Good, σp m p ∈ Decided.pages T Good') ∧
    (∀ p ∈ Decided.pages T Good', ∃ q ∈ Decided.pages T Good, σp m q = p) ∧
    (∀ p ∈ Decided.pages T Good, ∀ q ∈ Decided.pages T Good, σp m p = σp m q → p = q) :=
  ⟨fun _ hp => pagesR_rename_fwd good_hyps good_exceptionFree hp,
   fun _ hp => pagesR_rename_onto good_hyps good_exceptionFree hp,
   fun _ hp _ hq he => pagesR_rename_inj good_hyps hp hq he⟩

theorem good_favorites : Decided.renameState T R Good ["old", "old/ref", "p"] "old" "new" =
    some (Good', ["new", "new/ref", "p"]) := by decide +kernel

theorem good_link : T.Link true fav Good (.page, "c") (.page, "p") ∧
    T.Link true fav Good' (.page, "c") (.page, "p") := by
  have hr : T.Link true fav Good (.page, "c") (.page, "p") :=
    T.linkB_iff.mp (by decide +kernel)
  have hf : ∀ p ∈ T.pagesR true Good, σp m p = fav ↔ p = fav := by decide +kernel
  exact ⟨hr, (link_rename_favorites good_hyps good_exceptionFree hf
    (by decide +kernel) (by decide +kernel)).mp hr⟩

/- Minimal accepted merge, one real file: pages a,b merge into b and the edge
a → b becomes a self-link. All occurrences are covered; no conflict exists. -/
def Merge : Graph := [pg "a" "l:b"]
def mm : String → String := nsSubst T.key T.urlHasOrigin "a" "b"
def Merge' : Graph := T.rename R.base Merge "a" "b"
theorem merge_allowed_clean : Decided.renameAllowed T Merge "a" "b" = true ∧
    T.conflictFreeB Merge = true ∧ Decided.rename T R Merge "a" "b" = some Merge' := by decide +kernel

theorem merge_hyps : T.MergeHyps R.base mm mm Merge where
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

theorem merge_not_injective : ¬ (∀ a ∈ T.inPlayKeys Merge, ∀ b ∈ T.inPlayKeys Merge,
    mm a = mm b → a = b) := by decide +kernel

theorem merge_no_bijection : ¬ ∃ σ, Witnesses2.PageBij σ
    (Decided.pages T Merge) (Decided.pages T Merge') := by
  exact Witnesses2.no_bij_lt (A₀ := [(.page, "a"), (.page, "b")])
    (B₀ := [(.page, "b")]) (by decide +kernel) (by decide +kernel)
    (by decide +kernel) (by decide +kernel)

theorem merge_self_link : T.linkB true fav Merge (.page, "a") (.page, "b") = true ∧
    T.linkB true fav Merge' (σp mm (.page, "a")) (σp mm (.page, "b")) = false := by decide +kernel

theorem merge_exact_applies :
    T.References Merge' ("b", 0) (.page, "b") ↔
      ∃ o ∈ T.occAt Merge ("a", 0), ∃ u ∈ T.occClosure (rwOcc R.base.covers mm o).2,
        cls (Reach.mapE mm (T.aliasEdges Merge)) (T.key u) "b" :=
  references_merge_exact merge_hyps (f := pg "a" "l:b") (by decide +kernel) 0 (.page, "b")

theorem merge_forward_applies : T.References Merge' ("b", 0) (.page, "b") :=
  references_merge_forward merge_hyps (f := pg "a" "l:b") (by decide +kernel) 0
    (t := (.page, "b")) (by decide +kernel) (T.referencesB_iff _ _ _ |>.mp (by decide +kernel))

/- New namespace pages invalidate even the repaired blanket SURJECTION claim. -/
def Cross : Graph := [pg "x" "l:x"]
def Cross' : Graph := T.rename R.base Cross "x" "n/y"
theorem cross_allowed : Decided.renameAllowed T Cross "x" "n/y" = true ∧
    T.conflictFreeB Cross = true ∧
    T.referencesB Cross' ("n/y", 0) (.page, "n") = true := by decide +kernel

def mc : String → String := nsSubst T.key T.urlHasOrigin "x" "n/y"
theorem cross_hyps : T.MergeHyps R.base mc mc Cross where
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

theorem cross_new_namespace :
    ((.page, "n") : Page String) ∈ Decided.pages T Cross' ∧
    "n" ∈ T.rewrittenRefKeys R.base mc Cross ∧
    "n" ∉ T.inPlayKeys Cross := by decide +kernel

theorem cross_no_surjection : ¬ ∃ σ : Page String → Page String,
    ∀ q ∈ Decided.pages T Cross', ∃ p ∈ Decided.pages T Cross, σ p = q := by
  rintro ⟨σ, hs⟩
  have h := Witnesses2.length_le_of_surj σ [(.page, "n/y"), (.page, "n")]
    (A := [(.page, "x")]) (by decide +kernel) (by
      intro q hq
      obtain ⟨p, hp, he⟩ := hs q (by
        have hc : ∀ q ∈ [(.page, "n/y"), (.page, "n")], q ∈ Decided.pages T Cross' := by decide +kernel
        exact hc q hq)
      exact ⟨p, (show ∀ p ∈ Decided.pages T Cross, p ∈ [(.page, "x")] from by decide +kernel) p hp, he⟩)
  simp at h

/- C4 removed, C7 permanent (both task markers AND priorities). -/
def PropertyGraph : Graph := [pg "a" "", pg "s" "p:a"]
theorem property_repaired : Decided.renameAllowed T PropertyGraph "a" "b" = true ∧
    T.linkB true fav PropertyGraph (.page, "s") (.page, "a") = true ∧
    T.linkB true fav (T.rename R.base PropertyGraph "a" "b") (.page, "s") (.page, "b") = true := by decide +kernel

def Exceptions : Graph := [pg "todo" "", pg "s" "k:todo r:todo"]
def Exceptions' := T.rename R.base Exceptions "todo" "chores"
theorem permanent_exceptions : Decided.renameAllowed T Exceptions "todo" "chores" = true ∧
    T.referencesB Exceptions ("s", 0) (.page, "todo") = true ∧
    T.referencesB Exceptions' ("s", 0) (.page, "chores") = false ∧
    T.occAt Exceptions' ("s", 0) = [(.taskMarker, "todo"), (.priority, "todo")] ∧
    ((.page, "todo") : Page String) ∈ Decided.pages T Exceptions' := by decide +kernel

def PriorityGraph : Graph := [pg "A" "", pg "s" "r:A"]
theorem priority_exception : Decided.renameAllowed T PriorityGraph "A" "chores" = true ∧
    T.occAt (T.rename R.base PriorityGraph "A" "chores") ("s", 0) = [(.priority, "A")] ∧
    T.referencesB (T.rename R.base PriorityGraph "A" "chores") ("s", 0) (.page, "chores") = false := by
  decide +kernel

def AliasRepair : Graph := [pg "p" "a:old", pg "old" "", pg "c" "l:old"]
theorem alias_repaired : Decided.renameAllowed T AliasRepair "old" "new" = true ∧
    T.referencesB AliasRepair ("c", 0) (.page, "p") = true ∧
    T.referencesB (T.rename R.base AliasRepair "old" "new") ("c", 0) (.page, "p") = true ∧
    T.aliasEdges (T.rename R.base AliasRepair "old" "new") = [("p", "new")] := by decide +kernel

def FavoriteGraph : Graph := [pg "a" "l:c", pg "c" ""]
theorem favorite_membership_and_exclusion :
    Decided.renameState T R FavoriteGraph ["a"] "a" "favorites" =
      some (T.rename R.base FavoriteGraph "a" "favorites", ["favorites"]) ∧
    T.linkB true fav FavoriteGraph (.page, "a") (.page, "c") = true ∧
    T.linkB true fav (T.rename R.base FavoriteGraph "a" "favorites")
      (.page, "favorites") (.page, "c") = false := by decide +kernel

/- Non-injective substitution can merge alias concepts too, even though only
one real page moves and no real file is overwritten. -/
def AliasMerge : Graph := [pg "a" "a:x l:b"]
def AliasMerge' := T.rename R.base AliasMerge "a" "b"
theorem alias_merge_expands : Decided.renameAllowed T AliasMerge "a" "b" = true ∧
    T.conflictFreeB AliasMerge = true ∧
    sameCls (T.aliasEdges AliasMerge) "b" "x" = false ∧
    sameCls (T.aliasEdges AliasMerge') "b" "x" = true := by decide +kernel

theorem alias_merge_not_image : T.concept AliasMerge' "b" "x" ∧
    ¬ img mm (T.concept AliasMerge "b") "x" := by
  refine ⟨T.conceptB_iff _ _ _ |>.mp (by decide +kernel), ?_⟩
  rintro ⟨k, hk, he⟩
  rw [T.concept_eq_class] at hk
  have hs : ∀ e ∈ T.aliasEdges AliasMerge, e.1 ≠ "b" ∧ e.2 ≠ "b" := by decide +kernel
  have hkb : k = "b" := (cls_isolated hs k).mp hk
  rw [hkb] at he
  have hn : mm "b" ≠ "x" := by decide +kernel
  exact hn he

theorem alias_merge_refs_not_image : T.refs AliasMerge' ("b", 0) ≠
    (fun C => ∃ C₀, T.refs AliasMerge ("a", 0) C₀ ∧ C = img mm C₀) := by
  intro he
  have hr : T.refs AliasMerge' ("b", 0) (T.concept AliasMerge' "b") :=
    ⟨"b", by decide +kernel, rfl⟩
  rw [he] at hr
  obtain ⟨C₀, ⟨s, hs, rfl⟩, hc⟩ := hr
  have hnames : T.refNames AliasMerge ("a", 0) = ["b"] := by decide +kernel
  rw [hnames, List.mem_singleton] at hs
  subst s
  exact alias_merge_not_image.2 ((congrFun hc "x").mp alias_merge_not_image.1)

def AliasChild : Graph := [pg "old" "l:old/x", pg "p" "a:new/x"]
def AliasChild' := T.rename R.base AliasChild "old" "new"
theorem alias_child_eliminated : Decided.renameAllowed T AliasChild "old" "new" = true ∧
    ((.page, "old/x") : Page String) ∈ Decided.pages T AliasChild ∧
    σp m (.page, "old/x") = (.page, "new/x") ∧
    ((.page, "new/x") : Page String) ∉ Decided.pages T AliasChild' ∧
    T.resolve AliasChild' "new/x" = (.page, "p") := by decide +kernel

def AliasTarget : Graph := [pg "old" "a:old/x", pg "new/x" ""]
theorem rewritten_alias_can_target_real : Decided.renameAllowed T AliasTarget "old" "new" = true ∧
    T.aliasEdges (T.rename R.base AliasTarget "old" "new") = [("new", "new/x")] := by decide +kernel

/- A clean journal below old: this is not C8's refused shared-key conflict. -/
def JournalChild : Graph := [pg "old" "l:old/sub", ⟨"old/sub", .journals, .md, "t:old/sub"⟩]
def JournalChild' := T.rename R.base JournalChild "old" "new"
theorem journal_child : T.conflictFreeB JournalChild = true ∧
    Decided.renameAllowed T JournalChild "old" "new" = false ∧
    T.owner JournalChild' (.journal, "old/sub") = some ⟨"old/sub", .journals, .md, "t:old/sub"⟩ ∧
    T.referencesB JournalChild ("old", 0) (.journal, "old/sub") = true ∧
    T.referencesB JournalChild' ("new", 0) (.journal, "old/sub") = false := by decide +kernel

/- Merging can also make genuinely new paths between OTHER endpoints. -/
def MergePath : Graph := [pg "a" "l:t", pg "s" "l:b"]
def MergePath' := T.rename R.base MergePath "a" "b"
theorem merge_creates_path : Decided.renameAllowed T MergePath "a" "b" = true ∧
    ((.page, "t") : Page String) ∉ T.nearL true fav MergePath .out 2 [(.page, "s")] ∧
    ((.page, "t") : Page String) ∈ T.nearL true fav MergePath' .out 2 [(.page, "s")] := by decide +kernel

/- Q8/Q9 and Q619 recomputation: refs alias-of-preblock is now non-uniform too. -/
def GQ : Graph := [pg "a" "h p:type;l:b;d l:c", pg "b" "l:a", pg "p" "h a:x", pg "c" "l:x"]
theorem query_alias : T.leafBlock3 Q GQ (.refs "p") ("c", 0) = true ∧
    T.leafBlock3 Q GQ (.refs "x") ("c", 0) = true ∧
    T.leafPage Q GQ (.name "x") (.page, "p") = false ∧
    Decided.resolve T GQ "x" = (.page, "p") ∧
    ((.page, "x") : Page String) ∉ Decided.pages T GQ := by decide +kernel

def leaves : List (Leaf Witnesses2.GL) :=
  [.refs "a", .refs "b", .refs "p", .refs "x", .name "a", .name "b", .name "x",
   .page (.refs "a"), .page (.name "a"), .given (.hasProp "type"), .given (.content "h")]
theorem q619 : leaves.map (T.kindUniformB3 Q true GQ) =
    [false, true, false, false, false, true, true, true, true, true, false] := by decide +kernel

theorem query_uniform_applies :
    (Node.page (.page, "a") ∈ T.den3 Q true GQ .page (.leaf (.page (.name "a"))) ↔
     Node.block ("a", 0) ∈ T.den3 Q true GQ .block (.leaf (.page (.name "a")))) ∧
    Node.page (.page, "a") ∈ T.den3 Q true GQ .page (.leaf (.page (.name "a"))) := by
  refine ⟨query_uniform3 (P := T) Q (by unfold WF; decide +kernel) ?_
    (by decide +kernel) (by decide +kernel), by decide +kernel⟩
  intro l hl
  simp only [Query.leaves, List.mem_singleton] at hl
  subst l
  exact T.page_kindUniform3 Q (by unfold WF; decide +kernel) (.name "a")

/- W1/W2: unrelated nonempty pages, alias classes, a reference-only page,
blocks, and a class-expanded refs query survive repair of the duplicate. -/
def Bad : Graph := [pg "d1" "t:dup", pg "d2" "t:dup",
  pg "safe" "h a:aka;l:loose", pg "source" "l:aka"]
def Repaired : Graph := [pg "d1" "t:dup", pg "safe" "h a:aka;l:loose", pg "source" "l:aka"]
def region : List String := ["safe", "aka", "loose", "source"]

theorem bad_conflicts : T.conflictKeys Bad = ["dup"] ∧
    T.conflictFreeB Repaired = true ∧
    (region.all (fun k => !T.conflictedB Bad k)) = true := by decide +kernel

theorem conflict_distinguishes_files :
    T.conflictedB [pg "a" "", pg "a" ""] "a" = false ∧
    T.conflictedB [pg "a" "", ⟨"a", .journals, .md, "t:a"⟩] "a" = true ∧
    T.conflictFreeB [pg "p" "a:x a:p", pg "q" "a:x a:p"] = true ∧
    sameCls (T.aliasEdges [pg "p" "a:x a:p", pg "q" "a:x a:p"]) "p" "q" = true := by
  decide +kernel

theorem agreement_computes : Isolation.agreementB T region Bad Repaired = true := by decide +kernel

theorem conflict_set_applies (k : String) :
    k ∈ T.conflictKeys Bad ↔ k ∈ T.conflictKeys (Bad.reverse ++ Bad) :=
  T.conflictKeys_sameFiles (by intro f; simp) k

theorem bad_agreement : Isolation.Agreement T region Bad Repaired where
  files := by
    intro f hp
    have hprot : Isolation.protectedKeys T region Bad Repaired =
        ["safe", "aka", "loose", "source", "safe", "source", "safe", "source"] := by decide +kernel
    rw [hprot] at hp
    constructor
    · intro hf
      simp only [Bad, List.mem_cons, List.not_mem_nil, or_false] at hf
      rcases hf with rfl | rfl | rfl | rfl
      · decide +kernel
      · have hn : (T.claim (pg "d2" "t:dup")).2 ∉
            ["safe", "aka", "loose", "source", "safe", "source", "safe", "source"] := by decide +kernel
        exact False.elim (hn hp)
      · decide +kernel
      · decide +kernel
    · intro hf
      simp only [Repaired, List.mem_cons, List.not_mem_nil, or_false] at hf
      rcases hf with rfl | rfl | rfl <;> decide +kernel
  closedG := by unfold Isolation.Closed; decide +kernel
  closedH := by unfold Isolation.Closed; decide +kernel

theorem bad_pages : ((.page, "loose") : Page String) ∈ Decided.pages T Bad ∧
    ((.page, "loose") : Page String) ∈ Decided.pages T Repaired := by
  have hp : ((.page, "loose") : Page String) ∈ Decided.pages T Bad := by decide +kernel
  exact ⟨hp, (Isolation.pages_agree T lawful (by unfold WF; decide +kernel)
    (by unfold WF; decide +kernel) bad_agreement (by decide +kernel)).mp hp⟩

theorem bad_region_nonempty :
    T.resolve Bad "aka" = (.page, "safe") ∧ T.resolve Repaired "aka" = (.page, "safe") ∧
    T.pageBlocks Bad (.page, "safe") = [("safe", 0), ("safe", 1)] ∧
    T.pageBlocks Repaired (.page, "safe") = [("safe", 0), ("safe", 1)] ∧
    sameCls (T.aliasEdges Bad) "safe" "aka" = true ∧
    sameCls (T.aliasEdges Repaired) "safe" "aka" = true := by decide +kernel

def Q0 : QParams String Unit := { Q with blockLeaf := fun _ _ _ => false, pageLeaf := fun _ _ _ => false }
theorem bad_given : Isolation.GivenAgreement T Q0 region Bad Repaired :=
  ⟨fun _ _ _ => rfl, fun _ _ _ _ _ => rfl⟩
def safeQuery : Query Unit := .and (.leaf (.refs "safe")) (.not (.leaf (.name "dup")))
theorem bad_query :
    Node.block ("source", 0) ∈ Decided.den T Q0 Bad .block safeQuery ∧
    Node.block ("source", 0) ∈ Decided.den T Q0 Repaired .block safeQuery := by
  have hq : Node.block ("source", 0) ∈ Decided.den T Q0 Bad .block safeQuery := by decide +kernel
  refine ⟨hq, (Isolation.query_block_agree T Q0 lawful (by unfold WF; decide +kernel)
    (by unfold WF; decide +kernel) bad_agreement bad_given safeQuery ?_
    (p := (.page, "source")) (by decide +kernel) (by decide +kernel)).mp hq⟩
  intro l hl
  simp only [safeQuery, Query.leaves, List.mem_append, List.mem_singleton] at hl
  rcases hl with rfl | rfl
  · change T.key "safe" ∈ region; decide +kernel
  · trivial

theorem conflict_rename_refused : Decided.rename T R Bad "dup" "new" = none ∧
    Decided.renameAllowed T Bad "safe" "safe2" = true := by decide +kernel

def mb : String → String := nsSubst T.key T.urlHasOrigin "safe" "safe2"
theorem unrelated_conflicts_allow_transport : T.RenameHyps R.base mb mb Bad where
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

theorem journal_conflict_refused : Decided.rename T R Witnesses2.C8 "old" "new" = none := by decide +kernel

end Witnesses3
end CM
