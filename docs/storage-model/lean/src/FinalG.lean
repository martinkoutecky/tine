import AnchorUpdates
import GClosure3
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem anchors_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : Anchors x) (hv : VersionBounds x.s x.g.vc)
    (hc : CarrierAuth x) (hb : ByteDomain c x.s) (hw : WindowAuth x)
    (hen : enabled c x a = true) : Anchors (execute c x a) := by
  have he := evolution_execute c mutant x a
  cases a <;> simp only [execute] at he ⊢
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail => exact anchors_deliverUp c mutant x m tail ok h hv hq hc hb
  case wOpen p => exact anchors_wOpen c x p h
  case wClose p => exact anchors_wClose c x p h
  case wEdit p v => exact anchors_wEdit c x p v h
  case wDiscard p => exact anchors_wDiscard c x p h
  case wSend p => exact anchors_wSend c x p h hen.1.1.2
  case wResolve p v => exact anchors_wResolve c x p v h hen.1.1.1.2
  case wOp p q t u =>
    exact anchors_wOp c x p q t u h (by grind) (by grind)
  case wRecv p => exact anchors_wRecv c mutant x p h hb hw (by simpa [enabled] using hen)
  case check =>
    simp only [check] at he ⊢
    split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
    all_goals apply anchors_commit
    all_goals apply anchors_transport x _ _ h hv
    all_goals first | exact he | rfl | (intro m hm; exact hm) | exact authMono_refl x.g
  case draftSync p =>
    simp only [draftSync] at he ⊢
    split <;> (try split) <;> (try split)
    all_goals simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
    all_goals apply anchors_commit
    all_goals apply anchors_transport x _ _ h hv
    all_goals first | exact he | rfl | (intro m hm; exact hm) |
      exact authMono_refl x.g | exact authMono_wrote x.g _ _
  case windowCrash =>
    apply anchors_commit
    constructor
    · simp
    · simp
    · simp
    · intro m mem
      simp only [windowCrash] at mem
      split at mem
      · simp at mem
      · simp only [List.mem_map] at mem
        obtain ⟨u,hu,rfl⟩ := mem
        exact h.queue u hu
  case switchFin =>
    apply anchors_commit
    constructor <;> simp [QueueAnchors,down]
  case crash =>
    apply anchors_commit
    constructor <;> simp [QueueAnchors,down]
  case power keep =>
    apply anchors_commit
    constructor <;> simp [QueueAnchors,down]
  all_goals simp only [observe,flush,rename,dirSync,saveFail,switchReq,extWriteD,launch,
    evolution_commit,evolution_ackMail] at he ⊢
  all_goals apply anchors_commit
  all_goals apply anchors_transport x _ _ h hv
  all_goals first | exact he | rfl | (intro m hm; exact hm) |
    exact authMono_refl x.g | exact authMono_wrote x.g _ _

theorem anchors_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : Anchors x := by
  induction hx with
  | initial => exact anchors_init c
  | @advance x y hr a hs ih =>
    have hv := bounds_reachable c x hr
    have hc := carrier_reachable c x hr
    have hb := bytes_reachable c x hr
    have hw := windowAuth_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact anchors_execute c mutant x a ih hv hc hb hw (by assumption)
    · contradiction

theorem queueAnchors_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : QueueAnchors x := (anchors_reachable c mutant x hx).queue

/-- Exact G absence for arbitrary page/text counts and all race/fault profiles. -/
theorem G_positiveTexts (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts T) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : "G-overwrote-unseen" ∉ x.g.bad :=
  G_positiveTexts_of_queueAnchors c T texts mutant (queueAnchors_reachable c mutant) x hx

#print axioms anchors_execute
#print axioms anchors_reachable
#print axioms queueAnchors_reachable
#print axioms G_positiveTexts
end Storage
