import GRename6
import GObservations6
import CoreGDelivery6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

structure AuthorityInv6 (x : State P) : Prop where
  structural : GInv (shadow6 x)
  anchors : Anchors (shadow6 x)
  resolution : ResolutionAuth (shadow6 x)

def NoUnknown6 (c : Config P) : Prop := ∀ b, RealByte c b → b ≠ unknown

theorem positiveTexts_noUnknown6 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) : NoUnknown6 c := by
  intro b hb
  rcases hb with rfl | hb
  · decide
  · rw [texts] at hb
    obtain ⟨n,hn,rfl⟩ := List.mem_map.mp hb
    simp [unknown]; omega

theorem authority_old6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : AuthorityInv6 x)
    (bounds : VersionBounds x.s.toSys x.g.vc) (bytes : ByteDomain c x.s.toSys)
    (window : WindowSeen (project x)) (carrier : CarrierSeen (project x))
    (obs : ObsOnly c x.s.toSys) (noUnknown : NoUnknown6 c)
    (hen : enabled c x (.old a) = true) : AuthorityInv6 (execute c x (.old a)) := by
  have en : Storage21.enabled c (shadow6 x) a = true := hen
  have sys : (shadow6 (execute c x (.old a))).s = (Storage21.execute c (shadow6 x) a).s :=
    (old_sys6 c x a mutant).trans (shadow_sys_execute6 c x a).symm
  have guard : (shadow6 (execute c x (.old a))).g.guard = (Storage21.execute c (shadow6 x) a).g.guard :=
    (old_guard6 c x a mutant).trans (shadow_guard_execute6 c x a).symm
  have mono := shadow_old_auth6 c x a mutant
  have ca : CarrierAuth (shadow6 x) := by
    intro m mem o ho; left; exact carrier m mem o ho
  have wa : WindowAuth (shadow6 x) := by
    intro p on; left; exact window p on
  have gi : GInv (Storage21.execute c (shadow6 x) a) := by
    cases a with
    | deliverUp ok =>
      simp only [Storage21.execute]
      cases hq : (shadow6 x).s.up with
      | nil => exact h.structural
      | cons m tail => exact G_deliverUp_of_queueAnchors c mutant (shadow6 x) m tail ok h.structural hq h.anchors.queue h.resolution bytes
    | _ => exact G_execute_non_delivery c mutant (shadow6 x) _ h.structural en (by intro r; simp) (noUnknown _ (bytes.disk x.s.job.p))
  refine ⟨shadow_Gtransport6 _ _ gi sys guard mono,?_,?_⟩
  · exact shadow_anchors_transport6 _ _
      (anchors_execute c mutant (shadow6 x) a h.anchors bounds ca bytes wa en) sys mono
  · exact shadow_resolution_transport6 _ _ (resolutionAuth_execute c (shadow6 x) a h.resolution obs) sys mono

theorem authority_flushDel6 (c : Config P) (x : State P) (p : Fin P)
    (h : AuthorityInv6 x) (hen : enabled c x (.flushDel p) = true) :
    AuthorityInv6 (flushDel c x p) := by
  have held : (x.s.pages p).held = true := by simp [enabled] at hen; grind
  have eq : shadow6 (flushDel c x p) =
      {Storage21.flush c (shadow6 x) p with g := {(Storage21.flush c (shadow6 x) p).g with bad := []}} := by
    simp [shadow6,flushDel,lift,project,Storage21.flush,Storage21.commit]
  have mono : AuthMono (Storage21.flush c (shadow6 x) p).g (shadow6 (flushDel c x p)).g := fun _ _ hb => hb
  refine ⟨shadow_Gtransport6 _ _ (G_flush c (shadow6 x) p h.structural held) rfl rfl mono,?_,?_⟩
  · apply shadow_anchors_transport6 _ (Storage21.flush c (shadow6 x) p) _ rfl mono
    apply anchors_commit
    exact ⟨h.anchors.window,h.anchors.snapshot,h.anchors.took,h.anchors.queue⟩
  · apply shadow_resolution_transport6 _ (Storage21.flush c (shadow6 x) p) _ rfl mono
    exact resolutionAuth_commit c (shadow6 x) _ _ _ h.resolution

theorem authority_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : AuthorityInv6 x)
    (bounds : VersionBounds x.s.toSys x.g.vc) (bytes : ByteDomain c x.s.toSys)
    (window : WindowSeen (project x)) (carrier : CarrierSeen (project x))
    (obs : ObsOnly c x.s.toSys) (noUnknown : NoUnknown6 c)
    (hen : enabled c x a = true) : AuthorityInv6 (execute c x a) := by
  cases a with
  | old a => exact authority_old6 c x a mutant h bounds bytes window carrier obs noUnknown hen
  | opDelete p => exact ⟨G_delete6 c x p mutant h.structural,
      anchors_delete6 c x p mutant h.anchors bounds,resolution_delete6 c x p mutant h.resolution⟩
  | opRename src dst refs rt => exact ⟨G_rename_op6 c x src dst refs rt mutant h.structural hen,
      anchors_rename6 c x src dst refs rt mutant h.anchors bounds hen,
      resolution_rename6 c x src dst refs rt mutant h.resolution⟩
  | flushDel p => exact authority_flushDel6 c x p h hen

theorem authority_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) (noUnknown : NoUnknown6 c)
    (hr : Reachable c disk x) : AuthorityInv6 x := by
  induction hr with
  | initial =>
    constructor
    · simp [GInv,PageBases,DraftBases,JobBase,JobWritten,CheckedReal,shadow6,initWith,embed,Storage21.init,nojob,noneBytes]
    · constructor <;> simp [QueueAnchors,shadow6,initWith,embed,Storage21.init]
    · simp [ResolutionAuth,shadow6,initWith,embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact authority_execute6 c x a mutant ih (bounds_reachable4 c disk x mutant hr)
        (bytes_reachable4 c disk x mutant realDisk hr) (windowSeen_reachable4 c disk x mutant realDisk hr)
        (carrierSeen_reachable4 c disk x mutant realDisk hr) (obs_reachable6 c disk x mutant realDisk hr)
        noUnknown (by assumption)
    · contradiction

theorem publication_authority6 (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) (noUnknown : NoUnknown6 c) :
    GPublicationObligation5 c disk := by
  intro x hr hen
  have inv := (authority_reachable6 c disk x mutant realDisk noUnknown hr).structural
  have on : x.s.job.on = true := by simp [enabled,Storage21.enabled,project] at hen; grind
  have phase : x.s.job.phase = 2 := by simp [enabled,Storage21.enabled,project] at hen; grind
  have guarded := (C_reachable3 c disk x mutant hr).2 on phase
  have base := inv.2.2.2.1 on
  have real := inv.2.2.2.2.2 on phase guarded.1
  rcases base with unknown | equal | auth
  · exact False.elim (real unknown)
  · simp [JobAuthority,beq_iff_eq]; exact Or.inl (Or.inl (Or.inl equal.symm))
  · have actual := (shadow_auth6 x x.s.job.p x.s.job.base).mp auth
    simp [JobAuthority,Auth3_6,List.any_eq_true] at actual ⊢
    grind

theorem G_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) (noUnknown : NoUnknown6 c)
    (hr : Reachable c disk x) : "G-overwrote-unseen" ∉ x.g.bad :=
  (G_all_iff_publication5 c disk mutant).mpr (publication_authority6 c disk mutant realDisk noUnknown) x hr

theorem G_positiveTexts6 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (initDisk c) x) : "G-overwrote-unseen" ∉ x.g.bad :=
  G_reachable6 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts two)
    (positiveTexts_noUnknown6 c T texts) hr

theorem guarantee_positiveTexts6 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (initDisk c) x) : guarantee x = true :=
  (guarantee_remaining6 c T texts two mutant x hr).mpr (G_positiveTexts6 c T texts two mutant x hr)

end Storage3
