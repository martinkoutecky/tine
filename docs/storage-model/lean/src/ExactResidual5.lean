import TypedClosure5
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem G_rename_iff_authority5 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x)
    (hen : enabled c x (.old .rename) = true) :
    "G-overwrote-unseen" ∉ (rename c x).g.bad ↔
      "G-overwrote-unseen" ∉ x.g.bad ∧ JobAuthority x = true := by
  have on : x.s.job.on = true := by simp [enabled,Storage21.enabled,project] at hen; grind
  have phase : x.s.job.phase = 2 := by simp [enabled,Storage21.enabled,project] at hen; grind
  have guarded := (C_reachable3 c disk x mutant hr).2 on phase
  have guard : x.g.guard = true := guarded.1
  simp [rename,commit,Storage21.commit,JobAuthority,guard,project,mem_union_general,tags]
  grind

def GPublicationObligation5 (c : Config P) (disk : Fin P → Int) : Prop :=
  ∀ x, Reachable c disk x → enabled c x (.old .rename) = true → JobAuthority x = true

theorem G_all_iff_publication5 (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none") :
    (∀ x, Reachable c disk x → "G-overwrote-unseen" ∉ x.g.bad) ↔ GPublicationObligation5 c disk := by
  constructor
  · intro all x hr hen
    have next : Reachable c disk (rename c x) :=
      Reachable.advance hr (.old .rename) (by simp [step,hen,execute,executeOld])
    exact ((G_rename_iff_authority5 c disk x mutant hr hen).mp (all _ next)).2
  · intro authority x hr
    exact G_reachable3_of_job_authority c disk mutant authority x hr

-- Exact family closure needs only the two named residuals below; B has no hypothesis.
def ACustodyObligation5 (c : Config P) (disk : Fin P → Int) : Prop :=
  ∀ x, Reachable c disk x → noLoss x = true → ∀ a,
    enabled c x a = true → AResidualAction a → noLoss (execute c x a) = true

theorem A_all_iff_custody5 (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none") :
    (∀ x, Reachable c disk x → noLoss x = true) ↔ ACustodyObligation5 c disk := by
  constructor
  · intro all x hr _ a hen _
    exact all _ (Reachable.advance hr a (by simp [step,hen]))
  · intro residual x hr
    exact noLoss_reachable3_of_residual c disk mutant residual x hr

theorem guarantee_all_iff_residuals5 (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) :
    (∀ x, Reachable c disk x → guarantee x = true) ↔
      ACustodyObligation5 c disk ∧ GPublicationObligation5 c disk := by
  constructor
  · intro all
    constructor
    · apply (A_all_iff_custody5 c disk mutant).mp
      intro x hr; exact ((guarantee_remaining5 c disk x mutant realDisk hr).mp (all x hr)).1
    · apply (G_all_iff_publication5 c disk mutant).mp
      intro x hr; exact ((guarantee_remaining5 c disk x mutant realDisk hr).mp (all x hr)).2
  · rintro ⟨a,g⟩ x hr
    apply (guarantee_remaining5 c disk x mutant realDisk hr).mpr
    exact ⟨(A_all_iff_custody5 c disk mutant).mpr a x hr,
      (G_all_iff_publication5 c disk mutant).mpr g x hr⟩

end Storage3
