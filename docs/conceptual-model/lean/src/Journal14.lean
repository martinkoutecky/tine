/- Journal creation and outside renames read as independent snapshots. -/

import DateProofs14

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Dates K)

structure EnsureResult where
  graph : Graph
  file : Option File
  created : Bool
  conflicts : Graph
  deriving DecidableEq, Repr

/-- `fresh` is a proposed configured journal path/format, checked before use.
An existing claimant is reused even if the graph contains other conflicts. -/
def ensureJournal (G : Graph) (d : K) (fresh : File) : EnsureResult :=
  match openingFile P D G (.journal,d) with
  | some f => ⟨G,some f,false,claimants P D G (.journal,d)⟩
  | none =>
    if fresh.dir = .journals ∧ claim P D fresh = (.journal,d) ∧ fresh.text = "" ∧
        (G.map File.path).Nodup ∧ fresh.path ∉ G.map File.path then
      ⟨fresh :: G,some fresh,true,[fresh]⟩
    else ⟨G,none,false,[]⟩

theorem ensure_reuses {G : Graph} {d : K} {f : File}
    (h : openingFile P D G (.journal,d) = some f) (fresh : File) :
    ensureJournal P D G d fresh = ⟨G,some f,false,claimants P D G (.journal,d)⟩ := by
  simp [ensureJournal,h]

theorem ensure_original_files (G : Graph) (d : K) (fresh : File) {f : File} (hf : f ∈ G) :
    f ∈ (ensureJournal P D G d fresh).graph := by
  unfold ensureJournal
  split
  · exact hf
  · split <;> simp_all

theorem ensure_created_exact {G : Graph} {d : K} {fresh : File}
    (h : (ensureJournal P D G d fresh).created = true) :
    openingFile P D G (.journal,d) = none ∧ fresh.dir = .journals ∧
    claim P D fresh = (.journal,d) ∧ fresh.text = "" ∧ WF (fresh :: G) ∧
    (ensureJournal P D G d fresh).graph = fresh :: G := by
  unfold ensureJournal at h ⊢
  split at h
  · contradiction
  · next hn =>
    split at h
    · next hg =>
      simp only at h
      rw [hn,if_pos hg]
      exact ⟨rfl,hg.1,hg.2.1,hg.2.2.1,by exact List.nodup_cons.mpr ⟨hg.2.2.2.2,hg.2.2.2.1⟩,rfl⟩
    · contradiction

theorem opening_cons_fresh {G : Graph} {d : K} {fresh : File}
    (hn : openingFile P D G (.journal,d) = none)
    (hc : claim P D fresh = (.journal,d)) :
    openingFile P D (fresh :: G) (.journal,d) = some fresh := by
  have he : claimants P D G (.journal,d) = [] := minBy_eq_none.mp hn
  simp [openingFile,claimants,List.filter_cons,hc,show G.filter (fun f => decide (claim P D f = (.journal,d))) = [] from he,minBy]

theorem ensure_idempotent (G : Graph) (d : K) (fresh : File) :
    (ensureJournal P D (ensureJournal P D G d fresh).graph d fresh).graph =
      (ensureJournal P D G d fresh).graph := by
  cases ho : openingFile P D G (.journal,d) with
  | some f => simp [ensureJournal,ho]
  | none =>
    by_cases hg : fresh.dir = .journals ∧ claim P D fresh = (.journal,d) ∧ fresh.text = "" ∧
        (G.map File.path).Nodup ∧ fresh.path ∉ G.map File.path
    · have hn := opening_cons_fresh P D ho hg.2.1
      simp [ensureJournal,ho,hg,hn]
    · simp only [ensureJournal,ho,if_neg hg]

theorem ensure_any_claimant_reuses {G : Graph} {d : K} {f : File} (fresh : File)
    (hf : f ∈ G) (hc : claim P D f = (.journal,d)) :
    (ensureJournal P D G d fresh).graph = G ∧ (ensureJournal P D G d fresh).created = false := by
  cases ho : openingFile P D G (.journal,d) with
  | some x => simp [ensureJournal,ho]
  | none =>
    have hn : claimants P D G (.journal,d) = [] := minBy_eq_none.mp ho
    have hm : f ∈ claimants P D G (.journal,d) := by simp [claimants,hf,hc]
    rw [hn] at hm; contradiction

/-- There is no history input and no eligibility check for reading a graph.
The environment may supply any replacement snapshot, including outside moves. -/
def outsideSnapshot (_before after : Graph) : Graph := after

theorem outside_reads (before after : Graph) : outsideSnapshot before after = after := rfl

theorem outside_content (before : Graph) (f : File) (path : String) (dir : Dir) :
    ({f with path := path, dir := dir} : File).text = f.text := rfl

end Date13
end CM
