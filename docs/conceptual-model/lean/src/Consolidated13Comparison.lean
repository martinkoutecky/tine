/- Explicit comparison domains. These results compare effects, not candidate
cardinality: the dialog intentionally offers more choices. Exhaustiveness of
the structural difference inventory is not established by these lemmas. -/
import Consolidated13Safety

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 Rulings9
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : QueryRename6.StoredParser L)

/-- Independently stated causes, not inequality of old and new answers. -/
def differenceCases (G : Graph) (extras : List String) (old new : String) : List (String × Bool) :=
  let m := nsSubst P.key P.urlHasOrigin old new
  let ds := demands P SP G extras old (literal P G old new)
  [("alias destination/options", decide (P.key old ≠ P.key new) && P.declaredKey G (P.key new)),
   ("additional group choice", (groupFiles P G new).any (fun f =>
      decide ((P.claim f).2 ≠ P.key old ∧ (P.claim f).2 ≠ P.key new))),
   ("moving destination", (nsMatch P.key P.urlHasOrigin old new).isSome && decide (P.key old ≠ P.key new)),
   ("fixed anchor", (anchors P G).any (fun s => decide (P.key (m s) ≠ P.key s))),
   ("inconsistent sites", ds.any (fun a => ds.any (fun b => decide (a.1 = b.1 ∧ a.2 ≠ b.2)))),
   ("image conflict", (directNames P SP G).any (fun s =>
      (nsMatch P.key P.urlHasOrigin old s).isSome && P.conflictedB G (P.key (m s)))),
   ("journal closure loss", !(Decided.journalRewriteRisks P.toParams G old new).isEmpty),
   ("new source kind", decide (Rulings11.sourceKind P.toParams G old = .unsupported) && supported P G old),
   ("literal target spelling", (uniqueClaim P G (P.key new)).any (fun D => decide (P.name D ≠ new))),
   ("twins", decide (activeGraph P G ≠ G))]

theorem old_rename_shape (R : QRewriter) (J : (String → String) → Merger)
    {G H : Graph} {old new : String}
    (h : Rulings11.answer P.toParams R J G old new .rename = some H) :
    H = renameBy (R.at (nsSubst P.key P.urlHasOrigin old new)).base
      (nsSubst P.key P.urlHasOrigin old new) G := by
  change (if (Rulings11.offers P.toParams G old new).contains .rename then
    Rulings11.renameChosen P.toParams (R.at (nsSubst P.key P.urlHasOrigin old new)) G old new else none) = some H at h
  split at h
  · unfold Rulings11.renameChosen Ref10.rename10 Decided.rename at h
    split at h
    · split at h
      · exact (Option.some.inj h).symm
      · contradiction
    · split at h
      · split at h
        · exact (Option.some.inj h).symm
        · contradiction
      · split at h
        · exact (Option.some.inj h).symm
        · contradiction
  · contradiction

/-- Effect agreement whenever the old rename succeeds, the literal candidate
passes the new finite checks, and no root content pair is selected. The safety
premise is essential: W5/W9/W10 are old successes newly refused. -/
theorem default_rename_overlap (E : Engine) {G H : Graph} {extras : List String}
    {old new : String}
    (h : Rulings11.answer P.toParams E.rewrite E.merger G old new .rename = some H)
    (hn : (literal P G old new).survivor = none)
    (hs : safe P SP E G extras old (literal P G old new) = true) :
    defaultCandidate P SP E G extras old new = some (literal P G old new) ∧
    execute P E G old (literal P G old new) = H := by
  refine ⟨by simp [defaultCandidate, hs], ?_⟩
  rw [old_rename_shape P E.rewrite E.merger h]
  unfold execute
  rw [hn]
  cases source P G old <;> rfl

/-- Direct stationary merge domain. Matching source/target selection and one
literal spelling map are structural input conditions, not after-graph equality.
No claim is made that every old allowed merge satisfies these conditions. -/
theorem default_merge_overlap (E : Engine) {G : Graph} {extras : List String}
    {old new : String} {S D : File}
    (hs : source P G old = some S)
    (hd : (literal P G old new).survivor = some D)
    (hos : Merge7.mergeSource P.toParams G old = some S)
    (hot : Rulings11.target P.toParams G old new = some D)
    (hn : Rulings11.noFileSource P.toParams G old = false)
    (hr : Rulings11.mergeReady P.toParams G old new = true)
    (ho : .mergeOwner ∈ Rulings11.offers P.toParams G old new)
    (hm : queryMap P.toParams old new D = nsSubst P.key P.urlHasOrigin old new)
    (hc : safe P SP E G extras old (literal P G old new) = true) :
    defaultCandidate P SP E G extras old new = some (literal P G old new) ∧
    Rulings11.answer P.toParams E.rewrite E.merger G old new .mergeOwner =
      some (execute P E G old (literal P G old new)) := by
  refine ⟨by simp [defaultCandidate, hc], ?_⟩
  simp only [Rulings11.answer, List.contains_iff_mem, ho, ite_true]
  simp only [Rulings11.mergeChosen, hr, ite_true, hot, hn, Bool.false_eq_true,
    ite_false, hos, hm]
  unfold execute
  rw [hs, hd]
  rfl

end Consolidated13
end CM
