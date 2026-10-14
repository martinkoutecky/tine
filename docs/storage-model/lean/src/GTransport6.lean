import GShadow6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem old_guard6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).g.guard = (Storage21.execute c (project x) a).g.guard := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem shadow_Gtransport6 (y : State P) (z : Storage21.State P)
    (h : GInv z) (sys : (shadow6 y).s = z.s) (guard : (shadow6 y).g.guard = z.g.guard)
    (auth : AuthMono z.g (shadow6 y).g) : GInv (shadow6 y) := by
  rcases h with ⟨_,hp,hd,hj,hw,hc⟩
  refine ⟨by simp [shadow6],?_,?_,?_,?_,?_⟩
  · intro p held; have old := hp p (by simpa only [sys] using held)
    simpa only [sys] using admissible_mono auth old
  · intro p present; have old := hd p (by simpa only [sys] using present)
    simpa only [sys] using admissible_mono auth old
  · intro on; have old := hj (by simpa only [sys] using on)
    simpa only [sys] using admissible_mono auth old
  · intro on phase; have old := hw (by simpa only [sys] using on) (by simpa only [sys] using phase)
    simpa only [sys] using auth _ _ old
  · simpa only [CheckedReal,sys,guard] using hc

theorem shadow_anchors_transport6 (y : State P) (z : Storage21.State P)
    (h : Anchors z) (sys : (shadow6 y).s = z.s) (auth : AuthMono z.g (shadow6 y).g) :
    Anchors (shadow6 y) := by
  constructor
  · intro p on held hv
    have old := h.window p (by simpa only [sys] using on) (by simpa only [sys] using held) (by simpa only [sys] using hv)
    simpa only [sys] using auth _ _ old
  · simpa only [sys] using h.snapshot
  · intro p on took held hv
    have old := h.took p (by simpa only [sys] using on) (by simpa only [sys] using took)
      (by simpa only [sys] using held) (by simpa only [sys] using hv)
    simpa only [sys] using auth _ _ old
  · intro m mem; have old := h.queue m (by simpa only [sys] using mem)
    constructor
    · intro kind held hv
      have hp := old.1 kind (by simpa only [sys] using held) (by simpa only [sys] using hv)
      simpa only [sys] using auth _ _ hp
    · intro kind held hv
      have hp := old.2 kind (by simpa only [sys] using held) (by simpa only [sys] using hv)
      simpa only [sys] using auth _ _ hp

theorem shadow_resolution_transport6 (y : State P) (z : Storage21.State P)
    (h : ResolutionAuth z) (sys : (shadow6 y).s = z.s) (auth : AuthMono z.g (shadow6 y).g) :
    ResolutionAuth (shadow6 y) := by
  intro m mem ro; exact auth _ _ (h m (by simpa only [sys] using mem) ro)

end Storage3
