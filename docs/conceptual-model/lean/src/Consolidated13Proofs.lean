/- Finite checks imply transport, anchors and complete reference previews.
Writer laws are syntax/trace obligations; no semantic preservation is assumed. -/
import Consolidated13

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

theorem consistent_pair {ds : List (K × K)} (h : consistent ds = true)
    {a b : K × K} (ha : a ∈ ds) (hb : b ∈ ds) (he : a.1 = b.1) : a.2 = b.2 := by
  simp only [consistent, List.all_eq_true, decide_eq_true_eq] at h
  exact h a ha b hb he

theorem transport_realizes {ds : List (K × K)} (h : consistent ds = true)
    {a : K × K} (ha : a ∈ ds) : transport ds a.1 = a.2 := by
  unfold transport
  cases hf : ds.find? (fun b => decide (b.1 = a.1)) with
  | none =>
    have hn := List.find?_eq_none.mp hf
    have := hn a ha
    simp at this
  | some b =>
    have hb := List.mem_of_find?_eq_some hf
    have he : b.1 = a.1 := by simpa using List.find?_some hf
    simp only [Option.map_some, Option.getD_some]
    exact consistent_pair h hb ha he

theorem consistent_iff_function {ds : List (K × K)} :
    consistent ds = true ↔ ∃ σ : K → K, ∀ a ∈ ds, σ a.1 = a.2 := by
  constructor
  · intro h; exact ⟨transport ds, fun _ ha => transport_realizes h ha⟩
  · rintro ⟨σ, h⟩
    simp only [consistent, List.all_eq_true, decide_eq_true_eq]
    intro a ha b hb he
    exact (h a ha).symm.trans ((congrArg σ he).trans (h b hb))

theorem offered_safe {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∈ offers P SP E G extras old new) :
    safe P SP E G extras old c = true := (List.mem_filter.mp h).2

theorem offers_exact {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} : c ∈ offers P SP E G extras old new ↔
      c ∈ candidates P G old new ∧ safe P SP E G extras old c = true := by
  exact List.mem_filter

theorem default_offered {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : defaultCandidate P SP E G extras old new = some c) :
    c ∈ offers P SP E G extras old new := by
  change (if safe P SP E G extras old (literal P G old new) then
    some (literal P G old new) else none) = some c at h
  split at h
  · next hs =>
    cases Option.some.inj h
    apply List.mem_filter.mpr
    exact ⟨by simp [candidates], hs⟩
  · contradiction

theorem safe_consistent {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) :
    consistent (demands P SP G extras old c) = true := by
  simp only [safe, Bool.and_eq_true] at h
  exact h.1.1.1.1.1.1.2

theorem safe_wf {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) :
    WF G ∧ WF (execute P E G old c) := by
  simp only [safe, Bool.and_eq_true, decide_eq_true_eq] at h
  exact ⟨h.1.2, h.2⟩

theorem safe_root {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) :
    transport (demands P SP G extras old c) (P.key old) = P.key c.destination := by
  exact transport_realizes (safe_consistent P SP h) (a := (P.key old, P.key c.destination))
    (List.mem_cons_self ..)

theorem safe_anchor {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) {s : String}
    (hs : s ∈ anchors P G) :
    transport (demands P SP G extras old c) (P.key s) = P.key s := by
  apply transport_realizes (safe_consistent P SP h) (a := (P.key s, P.key s))
  unfold demands
  exact List.mem_cons_of_mem _ (List.mem_append_right _ (List.mem_map.mpr ⟨s, hs, rfl⟩))

theorem safe_site {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) {s : String}
    (hs : s ∈ directNames P SP G ++ extras) :
    transport (demands P SP G extras old c) (P.key s) = P.key (subst P old c s) := by
  apply transport_realizes (safe_consistent P SP h)
    (a := (P.key s, P.key (subst P old c s)))
  unfold demands
  exact List.mem_cons_of_mem _ (List.mem_append_left _ (List.mem_map.mpr ⟨s, hs, rfl⟩))

theorem cancel_identity (E : Engine) (G : Graph) (extras : List String) (old new : String) :
    answer P SP E G extras old new none = some G := rfl

theorem unoffered_refused {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∉ offers P SP E G extras old new) :
    answer P SP E G extras old new (some c) = none := by simp [answer, h]

theorem offered_executes {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {c : Candidate} (h : c ∈ offers P SP E G extras old new) :
    answer P SP E G extras old new (some c) = some (execute P E G old c) := by
  simp [answer, h]

theorem automatic_unique {E : Engine} {G : Graph} {extras : List String} {old new : String}
    {G' : Graph} (h : automatic P SP E G extras old new = some G') :
    ∃ c, offers P SP E G extras old new = [c] ∧ G' = execute P E G old c := by
  unfold automatic at h
  split at h
  · next c he => exact ⟨c, he, Option.some.inj h |>.symm⟩
  · contradiction

theorem fileless_cardinality (E : Engine) {G : Graph} {old : String} (c : Candidate)
    (h : source P G old = none) : (execute P E G old c).length = G.length := by
  simp [execute, h, renameBy]

theorem merge_exact_files (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {S D : File} (hs : source P G old = some S) (hd : c.survivor = some D) :
    execute P E G old c = (G.filter (fun f => decide (f ≠ S))).map
      (fun f => if f = D then (E.merger (subst P old c)).apply (subst P old c) S D
        else (rewriter E (subst P old c)).apply (subst P old c) f) := by
  simp [execute, hs, hd, mergeBy, Decided.Rewriter.base]

theorem signature_references {G H : Graph} {b b' : BlockId}
    (h : signature P G b = signature P H b') (t : Page K) :
    referencesB P G b t = referencesB P H b' t := by
  have repr : ∀ A x, referencesB P A x t =
      ((signature P A x).names.any (fun s => sameCls (signature P A x).edges (P.key s) t.2)) := by
    intro A x
    rfl
  rw [repr G b, repr H b', h]

theorem preview_complete (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {b b' : BlockId} (hb : b ∈ blocks P G) (hi : b' ∈ images P E G old c b)
    (t : Page K) (h : referencesB P G b t ≠ referencesB P (execute P E G old c) b' t) :
    (b, b') ∈ preview P E G old c := by
  have hn : signature P G b ≠ signature P (execute P E G old c) b' := by
    intro he; exact h (signature_references P he t)
  simp only [preview, List.mem_flatMap, List.mem_map, List.mem_filter, decide_eq_true_eq]
  exact ⟨b, hb, b', ⟨hi, hn⟩, rfl⟩

theorem lost_complete (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {b : BlockId} (hb : b ∈ blocks P G) (h : images P E G old c b = []) :
    b ∈ lost P E G old c := by simp [lost, hb, h]

end Consolidated13
end CM
