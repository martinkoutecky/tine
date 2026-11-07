/- Round 8b (verifier finding 2 on round 8): the alias-owner merge (M-Q5a)
where an alias spells the source's name. The verifier showed (its `VerifyR8`)
that round 8's `MergeBase … τ …` (alias contracts under the page map) has no
instance on these three accepted merges. With the split `MergeBase … τ … σ`
(alias contracts under the text map, `Merge7.aliasEdges_split`,
`Merge8.cls_split`) each is an instance, and MA2/MA3/MA4 apply. The graphs are
the verifier's (`Gother` with a referrer `c` added, `Gself`, `Gowner`).
Also the boundary of M-Q5(b) found by the verifier (finding 1), recorded for
Martin, not repaired. -/
import MergeWitness8

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace MergeWitness8b
open Params Merge7 Merge7b Merge8 MergeWitness7 MergeWitness8
open Witnesses2 (pg)

def Ss : File := pg "src" "a:src"
def Ow : File := pg "owner" "a:tgt a:src"

theorem via_of {S : File} (hS : (D.claim S).2 = "src") : ∀ k, k ≠ (D.claim S).2 → τ2 k = mm k := by
  intro k hk
  rw [hS] at hk
  unfold τ2; rw [if_neg hk]

/-! ### Go: a third page `u` declares `alias:: src`; afterwards it declares `alias:: tgt`, so `u`, `tgt` and `owner` are one class -/

def Go : Graph := [S2, O2, pg "u" "a:src", pg "c" "l:src"]

/-- **MW24 (accepted; hypotheses hold with the split maps).** The decided
merge resolves `tgt` to `owner` and accepts; the result is as shown; the
merge meets `MergeBodyHyps` with text map `mm` (alias contracts included) and
page map `τ2`; `ViaAlias` holds. -/
theorem mw_o_merge : mergeTarget D Go "src" "tgt" = some O2 ∧ mergeAllowed D Go "src" "tgt" = true ∧
    merge D R JT Go "src" "tgt" = some [pg "owner" "a:tgt;l:y", pg "u" "a:tgt", pg "c" "l:tgt"] ∧ mergeBy R.base JT mm S2 O2 Go = [pg "owner" "a:tgt;l:y", pg "u" "a:tgt", pg "c" "l:tgt"] := by
  decide +kernel

theorem mw_o_via : ViaAlias D mm τ2 S2 O2 Go where
  agree := via_of (by decide +kernel)
  declares := by decide +kernel
  fixed := by decide +kernel

theorem mw_o_hyps : MergeBodyHyps D Q SP R.base JT mm mm τ2 S2 O2 Go where
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
      have h1 : (D.outline S2.format S2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline O2.format O2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }
  lawsSD := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  sameFormat := by decide +kernel
  body := by decide +kernel
  queries := by decide +kernel
  storedOthers := by decide +kernel

/-- **MW25 (MA2 and MA4 on Go).** `c`'s link to `src` has an image that
references `owner`; `tgt` (the text image of `src`) is in `owner`'s class
afterwards. -/
theorem mw_o_refs :
    (∃ b' ∈ D.blocks (mergeBy R.base JT mm S2 O2 Go), Img R.base JT mm S2 O2 Go ("c", 0) b' ∧
      D.References (mergeBy R.base JT mm S2 O2 Go) b' (D.claim O2)) ∧
    cls (D.aliasEdges (mergeBy R.base JT mm S2 O2 Go)) (D.claim O2).2 "tgt" :=
  ⟨references_to_source_via mw_o_hyps.toMergeBase mw_o_via (by decide +kernel)
    (by unfold NsForward; decide +kernel) (D.referencesB_iff _ _ _ |>.mp (by decide +kernel)),
   class_join_via (x := "src") mw_o_hyps.toMergeBase mw_o_via (Or.inl (cls_self _ _))⟩

/-! ### Gs: the source declares its own name (`alias:: src`); the survivor lists `tgt` twice -/

def Gs : Graph := [Ss, O2, pg "c" "l:src"]

/-- **MW26 (accepted; hypotheses hold with the split maps).** The decided
merge resolves `tgt` to `owner` and accepts; the result is as shown; the
merge meets `MergeBodyHyps` with text map `mm` (alias contracts included) and
page map `τ2`; `ViaAlias` holds. -/
theorem mw_s_merge : mergeTarget D Gs "src" "tgt" = some O2 ∧ mergeAllowed D Gs "src" "tgt" = true ∧
    merge D R JT Gs "src" "tgt" = some [pg "owner" "a:tgt;a:tgt", pg "c" "l:tgt"] ∧ mergeBy R.base JT mm Ss O2 Gs = [pg "owner" "a:tgt;a:tgt", pg "c" "l:tgt"] := by
  decide +kernel

theorem mw_s_via : ViaAlias D mm τ2 Ss O2 Gs where
  agree := via_of (by decide +kernel)
  declares := by decide +kernel
  fixed := by decide +kernel

theorem mw_s_hyps : MergeBodyHyps D Q SP R.base JT mm mm τ2 Ss O2 Gs where
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
      have h1 : (D.outline Ss.format Ss.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline O2.format O2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }
  lawsSD := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  sameFormat := by decide +kernel
  body := by decide +kernel
  queries := by decide +kernel
  storedOthers := by decide +kernel

/-- **MW27 (MA2 and MA4 on Gs).** `c`'s link to `src` has an image that
references `owner`; `tgt` (the text image of `src`) is in `owner`'s class
afterwards. -/
theorem mw_s_refs :
    (∃ b' ∈ D.blocks (mergeBy R.base JT mm Ss O2 Gs), Img R.base JT mm Ss O2 Gs ("c", 0) b' ∧
      D.References (mergeBy R.base JT mm Ss O2 Gs) b' (D.claim O2)) ∧
    cls (D.aliasEdges (mergeBy R.base JT mm Ss O2 Gs)) (D.claim O2).2 "tgt" :=
  ⟨references_to_source_via mw_s_hyps.toMergeBase mw_s_via (by decide +kernel)
    (by unfold NsForward; decide +kernel) (D.referencesB_iff _ _ _ |>.mp (by decide +kernel)),
   class_join_via (x := "src") mw_s_hyps.toMergeBase mw_s_via (Or.inl (cls_self _ _))⟩

/-! ### Gw: the owner declares both the typed alias and the source's real name (`alias:: tgt, src`) -/

def Gw : Graph := [S2, Ow, pg "c" "l:src"]

/-- **MW28 (accepted; hypotheses hold with the split maps).** The decided
merge resolves `tgt` to `owner` and accepts; the result is as shown; the
merge meets `MergeBodyHyps` with text map `mm` (alias contracts included) and
page map `τ2`; `ViaAlias` holds. -/
theorem mw_w_merge : mergeTarget D Gw "src" "tgt" = some Ow ∧ mergeAllowed D Gw "src" "tgt" = true ∧
    merge D R JT Gw "src" "tgt" = some [pg "owner" "a:tgt a:tgt;l:y", pg "c" "l:tgt"] ∧ mergeBy R.base JT mm S2 Ow Gw = [pg "owner" "a:tgt a:tgt;l:y", pg "c" "l:tgt"] := by
  decide +kernel

theorem mw_w_via : ViaAlias D mm τ2 S2 Ow Gw where
  agree := via_of (by decide +kernel)
  declares := by decide +kernel
  fixed := by decide +kernel

theorem mw_w_hyps : MergeBodyHyps D Q SP R.base JT mm mm τ2 S2 Ow Gw where
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
      have h1 : (D.outline S2.format S2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline Ow.format Ow.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }
  lawsSD := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  sameFormat := by decide +kernel
  body := by decide +kernel
  queries := by decide +kernel
  storedOthers := by decide +kernel

/-- **MW29 (MA2 and MA4 on Gw).** `c`'s link to `src` has an image that
references `owner`; `tgt` (the text image of `src`) is in `owner`'s class
afterwards. -/
theorem mw_w_refs :
    (∃ b' ∈ D.blocks (mergeBy R.base JT mm S2 Ow Gw), Img R.base JT mm S2 Ow Gw ("c", 0) b' ∧
      D.References (mergeBy R.base JT mm S2 Ow Gw) b' (D.claim Ow)) ∧
    cls (D.aliasEdges (mergeBy R.base JT mm S2 Ow Gw)) (D.claim Ow).2 "tgt" :=
  ⟨references_to_source_via mw_w_hyps.toMergeBase mw_w_via (by decide +kernel)
    (by unfold NsForward; decide +kernel) (D.referencesB_iff _ _ _ |>.mp (by decide +kernel)),
   class_join_via (x := "src") mw_w_hyps.toMergeBase mw_w_via (Or.inl (cls_self _ _))⟩

/-- **MW30 (MA4 on `Go`: the third page joins the owner's class).** `u`
was in the source's class (it declared `alias:: src`); afterwards it is in
`owner`'s. -/
theorem mw_o_third : cls (D.aliasEdges (mergeBy R.base JT mm S2 O2 Go)) (D.claim O2).2 "u" :=
  class_join_via (x := "u") mw_o_hyps.toMergeBase mw_o_via
    (Or.inl ((sameCls_iff _ _ _).mp (by decide +kernel)))

/-- **MW32 (why the split: round 8's single-map form has no instance).** On
each of the three graphs no page map `τ` meets `MergeBase … τ …` with the
alias contracts under `τ` (round 8's MA1–MA4 hypotheses): the rewritten text
spells the old alias `src` as `tgt`, never as `owner`. (The verifier's
`no_base_other`/`no_base_self`/`no_base_owner`, restated here.) -/
theorem mw_no_single_map (τ : String → String) :
    ¬ MergeBase D R.base JT mm τ S2 O2 Go ∧ ¬ MergeBase D R.base JT mm τ Ss O2 Gs ∧
      ¬ MergeBase D R.base JT mm τ S2 Ow Gw := by
  refine ⟨fun H => ?_, fun H => ?_, fun H => ?_⟩
  · have ha := H.alias (pg "u" "a:src") (by decide +kernel) "src" (by decide +kernel)
    have hk : τ "src" = "owner" := H.srcKey
    change "tgt" = τ "src" at ha
    rw [hk] at ha
    exact (by decide : "tgt" ≠ "owner") ha
  · have hkτ : τ (D.key "src") = "owner" := H.srcKey
    have hm : "src" ∈ D.aliasDecl O2.format O2.text ++ D.aliasDecl Ss.format Ss.text := by
      decide +kernel
    obtain ⟨s, hs, hk⟩ := H.merger.aliasL.2 "src" hm
    have allTgt : ∀ s ∈ D.aliasDecl (JT.apply mm Ss O2).format (JT.apply mm Ss O2).text,
        D.key s = "tgt" := by decide +kernel
    rw [hkτ] at hk
    exact (by decide : "tgt" ≠ "owner") ((allTgt s hs).symm.trans hk)
  · have hkτ : τ (D.key "src") = "owner" := H.srcKey
    have hm : "src" ∈ D.aliasDecl Ow.format Ow.text ++ D.aliasDecl S2.format S2.text := by
      decide +kernel
    obtain ⟨s, hs, hk⟩ := H.merger.aliasL.2 "src" hm
    have allTgt : ∀ s ∈ D.aliasDecl (JT.apply mm S2 Ow).format (JT.apply mm S2 Ow).text,
        D.key s = "tgt" := by decide +kernel
    rw [hkτ] at hk
    exact (by decide : "tgt" ≠ "owner") ((allTgt s hs).symm.trans hk)

/-! ## Finding 1 (r8b open; closed in round 9 by CM-OWN-CHILD)

Round 9 (D-20261103-CMA): renaming or merging `a` onto a USED `a/x` (here an
alias declared by `o`) is refused. The round-8 acceptance below is kept as a
fact about the historical guard `renameAllowedR8` and the unguarded
`Params.rename`; the decided rename and merge now refuse. -/

def GaliasChild : Graph := [pg "a" "", pg "o" "a:a/x"]

/-- **MW31 (restated in round 9, ruling-driven).** Under the round-8 guard,
renaming `a` to `a/x`, where `a/x` is only an alias of `o`, was accepted, and
the rename map `a ↦ a/x` rewrote `o`'s `alias:: a/x` to `a/x/x`, leaving `a/x`
outside `o`'s class. Round 9 refuses it, for rename and for merge. -/
theorem mw_rename_alias_child :
    Decided.renameAllowedR8 D GaliasChild "a" "a/x" = true ∧
    D.rename R.base GaliasChild "a" "a/x" = [pg "a/x" "", pg "o" "a:a/x/x"] ∧
    D.aliasEdges GaliasChild = [("o", "a/x")] ∧
    D.aliasEdges [pg "a/x" "", pg "o" "a:a/x/x"] = [("o", "a/x/x")] ∧
    sameCls (D.aliasEdges [pg "a/x" "", pg "o" "a:a/x/x"]) "o" "a/x" = false ∧
    nsSubst D.key D.urlHasOrigin "a" "a/x" "a/x" = "a/x/x" ∧
    Decided.ownChildUsed D GaliasChild "a" "a/x" = true ∧
    Decided.rename D R GaliasChild "a" "a/x" = none ∧
    mergeAllowed D GaliasChild "a" "a/x" = false := by decide +kernel

end MergeWitness8b
end CM
