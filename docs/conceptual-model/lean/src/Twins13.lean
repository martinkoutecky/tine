/- Whole conflicting-group ownership transport, including cross-kind twins. -/
import Witness13Phase3

set_option maxHeartbeats 1600000
set_option maxRecDepth 8192
namespace CM
namespace Consolidated13
open Params QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

theorem uniqueClaim_spec {G : Graph} {k : K} {f : File}
    (h : uniqueClaim P G k = some f) :
    f ∈ G ∧ (P.claim f).2 = k ∧ ∀ g ∈ G, (P.claim g).2 = k → g = f := by
  unfold uniqueClaim at h
  generalize he : G.filter (fun f => decide ((P.claim f).2 = k)) = fs at h
  cases fs with
  | nil => simp at h
  | cons a as =>
    cases as with
    | cons b bs => simp at h
    | nil =>
      simp only [Option.some.injEq] at h
      subst a
      have hm : f ∈ G ∧ (P.claim f).2 = k := by
        have : f ∈ G.filter (fun f => decide ((P.claim f).2 = k)) := by rw [he]; simp
        simpa using this
      refine ⟨hm.1, hm.2, ?_⟩
      intro g hg hk
      have : g ∈ G.filter (fun f => decide ((P.claim f).2 = k)) := by simp [hg, hk]
      simpa [he] using this

theorem uniqueClaim_not_conflicted {G : Graph} {k : K} {f : File}
    (h : uniqueClaim P G k = some f) : P.conflictedB G k = false := by
  have hu := (uniqueClaim_spec P h).2.2
  cases hc : P.conflictedB G k
  · rfl
  · obtain ⟨a, ha, b, hb, hn, hak, hbk⟩ := P.conflictedB_iff.mp hc
    exact False.elim (hn ((hu a ha hak).trans (hu b hb hbk).symm))

theorem conflict_not_unique {G : Graph} {f : File} {k : K}
    (hc : P.conflictedB G (P.claim f).2 = true) : uniqueClaim P G k ≠ some f := by
  intro h
  have hk := (uniqueClaim_spec P h).2.1
  have hn := uniqueClaim_not_conflicted P h
  rw [← hk, hc] at hn
  contradiction

theorem conflict_not_source {G : Graph} {f : File} {old : String}
    (hc : P.conflictedB G (P.claim f).2 = true) : source P G old ≠ some f := by
  intro h
  unfold source at h
  exact conflict_not_unique P hc (Option.filter_eq_some_iff.mp h).1

theorem safe_pairReady {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) : pairReady P G old c = true := by
  simp only [safe, Bool.and_eq_true] at h
  exact h.1.1.1.1.2

theorem conflict_not_survivor {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P SP E G extras old c = true)
    {f : File} (hc : P.conflictedB G (P.claim f).2 = true) : c.survivor ≠ some f := by
  intro hd
  have hp := safe_pairReady P SP h
  unfold pairReady at hp
  rw [hd] at hp
  cases hs : source P G old with
  | none => simp [hs] at hp
  | some S =>
    simp only [hs, Bool.and_eq_true, decide_eq_true_eq] at hp
    exact conflict_not_unique P hc hp.1.2

theorem conflict_kept {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (_h : safe P SP E G extras old c = true)
    {f : File} (hf : f ∈ G) (hc : P.conflictedB G (P.claim f).2 = true) :
    f ∈ kept P G old c := by
  have hn := conflict_not_source P (old := old) hc
  unfold kept
  split
  · exact List.mem_filter.mpr ⟨hf, decide_eq_true hn⟩
  · exact hf

theorem conflict_output {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P SP E G extras old c = true)
    {f : File} (hc : P.conflictedB G (P.claim f).2 = true) :
    outputFile P E G old c f = (rewriter E (subst P old c)).apply (subst P old c) f := by
  have hn := conflict_not_survivor P SP h hc
  cases hs : source P G old with
  | none => simp only [outputFile, hs]
  | some S =>
    cases hd : c.survivor with
    | none => simp only [outputFile, hs, hd]
    | some D =>
      have hne : f ≠ D := by
        intro he
        subst D
        exact hn hd
      simp only [outputFile, hs, hd, if_neg hne]

theorem kept_subset (G : Graph) (old : String) (c : Candidate) :
    ∀ f ∈ kept P G old c, f ∈ G := by
  intro f hf
  unfold kept at hf
  split at hf
  · exact (List.mem_filter.mp hf).1
  · exact hf

theorem wf_path_injective {G : Graph} (h : WF G) {f g : File}
    (hf : f ∈ G) (hg : g ∈ G) (hp : f.path = g.path) : f = g := by
  induction G generalizing f g with
  | nil => simp at hf
  | cons a as ih =>
    simp only [WF, List.map_cons, List.nodup_cons] at h
    rcases List.mem_cons.mp hf with hfa | hft
    · rcases List.mem_cons.mp hg with hga | hgt
      · exact hfa.trans hga.symm
      · exact False.elim (h.1 (List.mem_map.mpr
          ⟨g, hgt, hp.symm.trans (congrArg File.path hfa)⟩))
    · rcases List.mem_cons.mp hg with hga | hgt
      · exact False.elim (h.1 (List.mem_map.mpr
          ⟨f, hft, hp.trans (congrArg File.path hga)⟩))
      · exact ih h.2 hft hgt hp

theorem minBy_map_order {α β : Type} (le : α → α → Bool) (le' : β → β → Bool)
    (F : α → β) (xs : List α)
    (h : ∀ a ∈ xs, ∀ b ∈ xs, le' (F a) (F b) = le a b) :
    minBy le' (xs.map F) = (minBy le xs).map F := by
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    have ht : ∀ x ∈ xs, ∀ y ∈ xs, le' (F x) (F y) = le x y :=
      fun x hx y hy => h x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy)
    simp only [List.map_cons, minBy, ih ht]
    cases hm : minBy le xs with
    | none => rfl
    | some b =>
      have hab := h a (List.mem_cons_self ..) b (List.mem_cons_of_mem _ (minBy_mem hm))
      cases he : le a b <;> simp [hab, he]

/-- Full ownership theorem: the same physical path opens after any safe
realized candidate at any previously conflicted key. No preservation premise. -/
theorem stationary_twins_opening {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old : String} {c : Candidate} {input : File → List Piece}
    (h : safe P SP E G extras old c = true) (F : Realizes P SP Q E G old c input)
    {k : K} (hc : P.conflictedB G k = true) :
    (openingFile P (execute P E G old c) k).map File.path =
      (openingFile P G k).map File.path := by
  let r := fun f => (rewriter E (subst P old c)).apply (subst P old c) f
  let fs := G.filter (fun f => decide ((P.claim f).2 = k))
  have facts : ∀ f ∈ fs, f ∈ kept P G old c ∧ outputFile P E G old c f = r f ∧
      (P.claim (r f)).2 = k ∧ (r f).path = f.path := by
    intro f hf
    obtain ⟨hf, hk⟩ := List.mem_filter.mp hf
    have hk : (P.claim f).2 = k := of_decide_eq_true hk
    have hc' : P.conflictedB G (P.claim f).2 = true := hk ▸ hc
    exact ⟨conflict_kept P SP h hf hc', conflict_output P SP h hc',
      (realized_conflict_key P SP h F hf hc').trans hk,
      realized_conflict_path P SP h F hf hc'⟩
  have same : ∀ x, x ∈ (execute P E G old c).filter (fun f => decide ((P.claim f).2 = k)) ↔
      x ∈ fs.map r := by
    intro x
    simp only [List.mem_filter, decide_eq_true_eq, List.mem_map]
    constructor
    · rintro ⟨hx, hk⟩
      have hm := execute_map_eq P E G old c
      unfold execute_map at hm
      rw [hm] at hx
      obtain ⟨g, hg, rfl⟩ := List.mem_map.mp hx
      obtain ⟨a, ha, b, hb, _, hak, _⟩ := P.conflictedB_iff.mp hc
      have ha' : a ∈ fs := List.mem_filter.mpr ⟨ha, decide_eq_true hak⟩
      have fa := facts a ha'
      have hgk := safe_no_new_claim_collision P SP h hg fa.1 (hk.trans (fa.2.1 ▸ fa.2.2.1).symm)
      have hgf : g ∈ fs := List.mem_filter.mpr
        ⟨kept_subset P G old c g hg, decide_eq_true (hgk.trans hak)⟩
      exact ⟨g, hgf, (facts g hgf).2.1.symm⟩
    · rintro ⟨f, hf, rfl⟩
      have ff := facts f hf
      constructor
      · have hm := execute_map_eq P E G old c
        unfold execute_map at hm
        rw [hm, ← ff.2.1]
        exact List.mem_map_of_mem ff.1
      · exact ff.2.2.1
  have order : ∀ a ∈ fs, ∀ b ∈ fs, claimantLe P (r a) (r b) = claimantLe P a b := by
    intro a ha b hb
    obtain ⟨haG, hak⟩ := List.mem_filter.mp ha
    obtain ⟨hbG, hbk⟩ := List.mem_filter.mp hb
    have hak : (P.claim a).2 = k := of_decide_eq_true hak
    have hbk : (P.claim b).2 = k := of_decide_eq_true hbk
    by_cases he : a.path = b.path
    · have he' := wf_path_injective (safe_wf P SP h).1 haG hbG he
      subst b
      have refl := fun f => (claimantLe_linear P).total f f |>.elim id id
      rw [refl, refl]
    · exact stationary_twins_order P SP h F haG hbG he (hak ▸ hc) (hak.trans hbk.symm)
  unfold openingFile
  rw [minBy_congr (claimantLe_linear P) same, minBy_map_order _ _ r fs order]
  cases hm : minBy (claimantLe P) fs with
  | none => rfl
  | some f =>
    exact congrArg some (facts f (minBy_mem hm)).2.2.2

theorem offered_twins_opening {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old new : String} {c : Candidate} {input : File → List Piece}
    (h : c ∈ offers P SP E G extras old new) (F : Realizes P SP Q E G old c input)
    {k : K} (hc : P.conflictedB G k = true) :
    (openingFile P (execute P E G old c) k).map File.path =
      (openingFile P G k).map File.path :=
  stationary_twins_opening P SP (offered_safe P SP h) F hc

end Consolidated13
end CM

namespace CM
namespace Witness13Order
open Params Consolidated13

abbrev P := Witness13.P

theorem path_before_format :
    claimantLe P ⟨"pages/a.org", .pages, .org, "t:U"⟩
      ⟨"pages/z.md", .pages, .md, "t:U"⟩ = true := by decide +kernel

theorem path_bytes_not_components :
    claimantLe P ⟨"pages/a-/z.md", .pages, .md, "t:U"⟩
      ⟨"pages/a/z.md", .pages, .md, "t:U"⟩ = true := by decide +kernel

theorem across_kinds :
    let J : File := ⟨"journals/day.md", .journals, .md, ""⟩
    let O : File := ⟨"pages/day.md", .pages, .md, ""⟩
    openingFile { P with stem := fun _ => "day", kindPref := .page } [J, O] "day" = some O ∧
    openingFile { P with stem := fun _ => "day", kindPref := .journal } [O, J] "day" = some J := by decide +kernel

/-- Actual allowed rename and merge each rewrite a twin's alias while retaining
the same physical active twin. Generic realization is covered above. -/
def twins : Graph := [Witnesses2.pg "A" "root",
  Witnesses2.pg "a.md" "t:U|a:A", ⟨"z.org", .pages, .org, "t:U|a:C"⟩]
def merging : Graph := twins ++ [Witnesses2.pg "Z" "target"]

theorem offered_rename_and_merge :
    let T := Witness13.P
    let E := Witness13.E
    let SP := Witness13.SP
    let c := literal T twins "A" "Z"
    let d := literal T merging "A" "Z"
    c ∈ offers T SP E twins [] "A" "Z" ∧
    d ∈ offers T SP E merging [] "A" "Z" ∧ d.survivor.isSome = true ∧
    (openingFile T twins "U").map File.path = some "a.md" ∧
    (openingFile T (execute T E twins "A" c) "U").map File.path = some "a.md" ∧
    (openingFile T (execute T E merging "A" d) "U").map File.path = some "a.md" := by decide +kernel

end Witness13Order
end CM
