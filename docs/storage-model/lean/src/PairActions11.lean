import PairApply11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
theorem pairSafety_delete11 (c : Config P) (x : State P) (p : Fin P) (h : PairSafety10 x) :
    PairSafety10 (execute c x (.prior (.opDelete p))) := by
  apply pairSafety_endpoints11 x (execute c x (.prior (.opDelete p))) h rfl
  · intro r mem; exact (List.mem_filter.mp mem).1
  · intro r mem
    have ne : r.1 ≠ p ∧ r.2 ≠ p := by simpa [unrename] using (List.mem_filter.mp mem).2
    constructor
    · simpa [execute,Storage35Base.execute,Storage32.execute,Storage32.opDelete,Storage32.lift,
        Storage3.opDelete,Storage3.commit,Storage21.commit,Storage34.lift,put,ne.1,project,Storage34.project,Storage32.project] using pageSafe_refl11 (x.s.disk r.1) (x.s.pages r.1)
    · simpa [execute,Storage35Base.execute,Storage32.execute,Storage32.opDelete,Storage32.lift,
        Storage3.opDelete,Storage3.commit,Storage21.commit,Storage34.lift,put,ne.2,project,Storage34.project,Storage32.project] using pageSafe_refl11 (x.s.disk r.2) (x.s.pages r.2)
  · exact fun _ hm => hm
  · rfl

theorem pairSafety_load11 (c : Config P) (x : State P) (p : Fin P) (h : PairSafety10 x)
    (en : enabled c x (.prior (.load p)) = true) : PairSafety10 (execute c x (.prior (.load p))) := by
  have off : (x.s.pages p).held = false := by
    simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,project,Storage34.project,Storage32.project] at en; grind
  apply pairSafety_frame11 x (execute c x (.prior (.load p))) h rfl (fun _ hm => hm)
  · simp only [execute,Storage35Base.execute,Storage32.execute,Storage32.lift,Storage31.execute,
      Storage31.load,Storage3.commit,Storage21.commit,Storage34.lift]
    apply pageSafe_put11
    · exact fun q => pageSafe_refl11 _ _
    · simp [PageSafe11,off,project,Storage34.project,Storage32.project]
  · exact fun _ hm => hm
  · exact fun on => ⟨on,rfl,rfl⟩

def externalView11 (x : State P) (p : Fin P) (b : Int) : State P :=
  {x with s := {x.s with disk := put x.s.disk p b},g := {x.g with renamed := unrename x.g.renamed [p]}}
theorem pairSafety_externalView11 (x : State P) (p : Fin P) (b : Int) (h : PairSafety10 x) :
    PairSafety10 (externalView11 x p b) := by
  intro r mem
  have old := List.mem_filter.mp mem
  have ne : r.1 ≠ p ∧ r.2 ≠ p := by simpa [unrename] using old.2
  have stable (q : Fin P) (neq : q ≠ p) (hp : StableSide10 x q) : StableSide10 (externalView11 x p b) q :=
    ⟨by simpa [externalView11,put,neq] using hp.1,hp.2⟩
  rcases h r old.1 with src | dst | wait
  · exact Or.inl (stable r.1 ne.1 src)
  · exact Or.inr (Or.inl (stable r.2 ne.2 dst))
  · exact Or.inr (Or.inr ⟨by simpa [externalView11,put,ne.1] using wait.1,wait.2⟩)
theorem pairSafety_external11 (c : Config P) (x : State P) (p : Fin P) (b : Int) (dur : Bool)
    (h : PairSafety10 x) : PairSafety10 (execute c x (.prior (.old (.extWriteD p b dur)))) := by
  apply (pairSafety_view11 _ (externalView11 x p b) ?_).mpr (pairSafety_externalView11 x p b h)
  exact ⟨rfl,rfl,rfl,rfl,rfl,rfl,rfl⟩

theorem pairSafety_delivery11 (c : Config P) (x : State P) (ok : Bool)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (own : GateOwnership11 x.s.gates)
    (kinds : Storage3.Kinds (core11 x).s) (inputs : InputTexts11 x.s.w x.s.up) :
    PairSafety10 (execute c x (.prior (.old (.deliverUp ok)))) := by
  cases hq : x.s.up with
  | nil => simpa [execute,Storage35Base.execute,Storage35Base.deliverUp,project,Storage34.project,hq,Storage34.lift] using h
  | cons m tail =>
    have kind := kinds m (by simpa [core11,project,Storage34.project,Storage32.project,Storage3.project,hq])
    have text := inputs.2 m (by simp [hq])
    simp only [execute,Storage35Base.execute,Storage35Base.deliverUp,project,Storage34.project,hq]
    split
    · exact pairSafety_discard11 c x m tail ok mutant own h
    · have nd : m.kind ≠ "discard" := by simpa using (by assumption : ¬(m.kind == "discard") = true)
      apply (pairSafety_view11 _ (coreLift11 x (Storage21.deliverUp c (core11 x) m tail ok)) ?_).mpr
        (pairSafety_core_delivery11 c x m tail ok mutant h kind text nd)
      simp [PairViewEq11,coreLift11,core11,Storage32.lift,Storage3.executeOld,Storage3.lift,mutant,
        Storage21.execute,project,Storage34.project,Storage32.project,Storage3.project,hq,Storage34.lift]

theorem pairSafety_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (texts : absent ∉ c.texts) (h : PairSafety10 x)
    (own : GateOwnership11 x.s.gates) (published : PairPublished11 (project x))
    (live : LivePairs11 x) (kinds : Storage3.Kinds (core11 x).s)
    (inputs : InputTexts11 x.s.w x.s.up) (en : enabled c x a = true) : PairSafety10 (execute c x a) := by
  cases a with
  | extClaim p present => exact h
  | opApply eff plan dst src => exact pairSafety_apply11 c x eff plan dst src mutant h en
  | prior a =>
    cases a with
    | load p => exact pairSafety_load11 c x p h en
    | opDelete p => exact pairSafety_delete11 c x p h
    | opRename src dst refs rt => exact pairSafety_opRename11 c x src dst refs rt mutant texts h en
    | flushDel p => exact pairSafety_flush11 c x p true mutant h own en
    | old a =>
      by_cases frame : CoreOnly11 a
      · exact pairSafety_oldFrame11 c x a mutant h frame
      cases a <;> try exact False.elim (frame trivial)
      case deliverUp ok => exact pairSafety_delivery11 c x ok mutant h own kinds inputs
      case flush p => exact pairSafety_flush11 c x p false mutant h own en
      case rename =>
        have on : x.s.job.on = true := by
          simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
            Storage21.enabled,project,Storage34.project,Storage32.project,Storage3.project] at en; grind
        exact pairSafety_fileRename11 c x mutant h on
      case dirSync ok => exact pairSafety_dirSync11 c x ok mutant h published en
      case extWriteD p b dur => exact pairSafety_external11 c x p b dur h
      case crash => simp [PairSafety10,execute,Storage35Base.execute,Storage32.execute,Storage32.stopped,Storage34.lift]
      case switchFin => simp [PairSafety10,execute,Storage35Base.execute,Storage32.execute,Storage32.stopped,Storage34.lift]
      case power keep => simp [PairSafety10,execute,Storage35Base.execute,Storage32.execute,Storage32.stopped,Storage34.lift]
      case launch =>
        have off : x.s.alive = false := by
          simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
            Storage21.enabled,project,Storage34.project,Storage32.project,Storage3.project] at en; grind
        have pairs : (execute c x (.prior (.old .launch))).g.renamed = [] := live off
        simp [PairSafety10,pairs]
end Storage35
