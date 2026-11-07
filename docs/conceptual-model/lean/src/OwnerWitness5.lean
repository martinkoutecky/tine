/- Kernel fixtures for CI4. The inherited toy parser uses t:/a: tokens;
without a title, its stem is the entire path. No production-parser claim. -/
import Owner5

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000

namespace CM
namespace OwnerWitness5
open Params
open Witnesses2 (pg)

def base : Params String := Witnesses3.T
def dateKey : String → Bool := Witnesses5.dateKey

def NamesDisagree : Graph := [pg "pages/Z.md" "t:Alpha a:x",
  pg "pages/A.md" "t:Zulu a:x"]

/-- CI4 is a proper refinement: the formerly possible name-order result differs. -/
theorem path_beats_name :
    (Policy5.params base dateKey).resolve NamesDisagree "x" = (.page, "Alpha") ∧
    Owner5.resolve base dateKey NamesDisagree "x" = (.page, "Zulu") := by decide +kernel

theorem enumeration_duplicates :
    Owner5.resolve base dateKey NamesDisagree "x" =
      Owner5.resolve base dateKey (NamesDisagree.reverse ++ NamesDisagree) "x" :=
  Owner5.T1_set base dateKey Witnesses3.lawful (by intro f; simp) "x"

theorem byte_examples :
    Owner5.bytes "journals/day.md" < Owner5.bytes "pages/A.md" ∧
    Owner5.bytes "pages/Beta.md" < Owner5.bytes "pages/alpha.md" ∧
    Owner5.bytes "pages/a b.md" < Owner5.bytes "pages/a/x.md" ∧
    Owner5.bytes "pages/z.md" < Owner5.bytes "pages/é.md" := by decide +kernel

/-- Round 9 (CM-NAMES (2)): the journal's page is named by its stem (this toy's
stem is the whole path), not by its `t:Day` title; round 8 expected
`(.journal, "Day")`. The path order, which this witness is about, is unchanged. -/
theorem journal_path_first :
    Owner5.resolve base dateKey
      [pg "pages/A.md" "t:Alpha a:x", ⟨"journals/day.md", .journals, .md, "t:Day a:x"⟩]
      "x" = (.journal, "journals/day.md") := by decide +kernel

theorem direct_only :
    Owner5.resolve base dateKey [pg "pages/A.md" "t:A a:B", pg "pages/Z.md" "t:B a:x"]
      "x" = (.page, "B") := by decide +kernel

/-- Model real-file priority is retained for a file with content. O5's
application-conformance status is not altered by this model fixture.
Round 9 (EMPTY-ALIAS, D-20261103-CMA): round 8 stated this for the EMPTY file
`pg "x" ""`; that statement is now false (the declaring owner wins, see
`empty_alias_yields`), so the witness carries content. -/
theorem empty_real_wins :
    Owner5.resolve base dateKey (pg "x" "body" :: NamesDisagree) "x" = (.page, "x") :=
  by decide +kernel

/-- Round 9 (EMPTY-ALIAS): the content-free file `x` yields to the owner that
declares `alias:: x` (here the CI4 owner `Zulu`, at the smaller path). -/
theorem empty_alias_yields :
    Owner5.resolve base dateKey (pg "x" "" :: NamesDisagree) "x" = (.page, "Zulu") :=
  by decide +kernel

def Before : Graph := [pg "pages/A.md" "a:x", pg "pages/B.md" "a:x"]
def move : String → String := nsSubst base.key base.urlHasOrigin "pages/A.md" "pages/Z.md"
def After : Graph := renameBy Witnesses3.R.base move Before

/-- A literal accepted rename changes the winner. Even mapping the former
winner's name is NOT a navigation-preservation theorem. -/
theorem rename_changes_owner :
    WF Before ∧ WF After ∧
    Decided.rename (Owner5.params base dateKey Before) Witnesses3.R Before
      "pages/A.md" "pages/Z.md" = some After ∧
    Owner5.resolve base dateKey Before "x" = (.page, "pages/A.md") ∧
    Owner5.resolve base dateKey After "x" = (.page, "pages/B.md") ∧
    Owner5.resolve base dateKey After "x" ≠
      σp move (Owner5.resolve base dateKey Before "x") := by
  unfold WF
  decide +kernel

/-- Fixed means deterministic at a state, not a sticky owner across a path move. -/
theorem stale_snapshot_wrong :
    (Owner5.params base dateKey Before).resolve After "x" ≠
      Owner5.resolve base dateKey After "x" := by decide +kernel

/-- Arbitrary timestamp annotations are discarded at the model boundary.
All modeled file fields and parameters are held fixed. -/
theorem timestamps_irrelevant (G : Graph) (t₁ t₂ : File → Nat) (s : String) :
    Owner5.resolve base dateKey ((G.map (fun f => (f, t₁ f))).map Prod.fst) s =
      Owner5.resolve base dateKey ((G.map (fun f => (f, t₂ f))).map Prod.fst) s := by
  simp only [List.map_map, Function.comp_def]

end OwnerWitness5
end CM

open Lean Elab Command in
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [
    ``CM.Owner5.bytes_injective, ``CM.Owner5.rankLe_iff, ``CM.Owner5.rankLe_linear,
    ``CM.Owner5.fold_instance, ``CM.Owner5.lawful, ``CM.Owner5.params_sameFiles,
    ``CM.Owner5.T1_set, ``CM.Owner5.T1_key, ``CM.Owner5.concept_erasure,
    ``CM.Owner5.references_erasure, ``CM.Owner5.structure_erasure,
    ``CM.Owner5.pageLeaf_erasure, ``CM.Owner5.blockLeaf_erasure,
    ``CM.Owner5.query_erasure, ``CM.Owner5.link_erasure, ``CM.Owner5.rename_erasure,
    ``CM.Owner5.declarer_owner, ``CM.Owner5.CI4, ``CM.Owner5.owner_rank_injective,
    ``CM.Owner5.minimum_unique, ``CM.Owner5.minBy_rank_congr,
    ``CM.Owner5.resolve_retarget, ``CM.Owner5.T4_resolve, ``CM.Owner5.W2_resolve,
    ``CM.OwnerWitness5.path_beats_name, ``CM.OwnerWitness5.enumeration_duplicates,
    ``CM.OwnerWitness5.byte_examples, ``CM.OwnerWitness5.journal_path_first,
    ``CM.OwnerWitness5.direct_only, ``CM.OwnerWitness5.empty_real_wins,
    ``CM.OwnerWitness5.empty_alias_yields,
    ``CM.OwnerWitness5.rename_changes_owner, ``CM.OwnerWitness5.stale_snapshot_wrong,
    ``CM.OwnerWitness5.timestamps_irrelevant]
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
  logInfo m!"ROUND 5b CI4 POLICY + WITNESS CONE: {seen.size} constants; axioms {axs.toList}"
