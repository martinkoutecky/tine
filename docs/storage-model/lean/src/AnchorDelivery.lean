import Anchors
import ProofTaken
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem anchors_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail) :
    Anchors (upOpen c x {x.s with up := tail} m ok) := by
  have he := evolution_upOpen c x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upOpen,evolution_commit,evolution_ackMail] at he
  simp only [upOpen]
  apply anchors_commit
  apply anchors_ackMail
  · apply anchors_transport x _ _ h hv
    · exact he
    · rfl
    · rfl
    · intro u hu; simp [hq,hu]
    · intro p b hb; split <;> (try split) <;> exact hb
  · simp

theorem anchors_upClose (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail) :
    Anchors (upClose c x {x.s with up := tail} m) := by
  have he := evolution_upClose c x {x.s with up := tail} m (evolution_refl x)
  simp only [upClose,evolution_commit] at he
  simp only [upClose]
  apply anchors_commit
  split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
  all_goals apply anchors_transport x _ _ h hv
  all_goals first | exact he | rfl | (intro u hu; simp [hq,hu]) | exact authMono_refl x.g

theorem anchors_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail) :
    Anchors (upDiscard c x {x.s with up := tail} m ok) := by
  have he := evolution_upDiscard c x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upDiscard,evolution_commit,evolution_ackMail] at he
  simp only [upDiscard]; split
  · apply anchors_commit
    apply anchors_transport x _ _ h hv
    · exact (evolution_refl x)
    · rfl
    · rfl
    · intro u hu; simp [hq,hu]
    · exact authMono_refl x.g
  · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
    all_goals apply anchors_commit
    all_goals apply anchors_ackMail
    all_goals first
      | (apply anchors_transport x _ _ h hv
         · exact he
         · rfl
         · rfl
         · intro u hu; simp [hq,hu]
         · exact authMono_refl x.g)
      | simp

theorem anchors_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P))
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail)
    (ht : Auth x.g m.p m.t) : Anchors (upSubmit c x {x.s with up := tail} m) := by
  have he := evolution_upSubmit c mutant x {x.s with up := tail} m (evolution_refl x)
  simp only [upSubmit,mutant,show ("none" == "MDO") = false from rfl,
    show ("none" != "MIS") = true from rfl,Bool.and_true,Bool.false_eq_true,ite_false,
    evolution_commit,evolution_ackMail] at he ⊢
  split
  · apply anchors_commit
    apply anchors_transport x _ _ h hv
    · exact (evolution_refl x)
    · rfl
    · rfl
    · intro u hu; simp [hq,hu]
    · exact authMono_refl x.g
  · simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
    apply anchors_commit
    apply anchors_ackMail
    · apply anchors_transport x _ _ h hv
      · exact he
      · rfl
      · rfl
      · intro u hu; simp [hq,hu]
      · exact authMono_refl x.g
    · intro _ _
      dsimp only [Auth] at ht ⊢
      simp only [put,ite_true]
      split <;> simpa only [submitTo_takes c mutant] using ht

theorem anchors_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail)
    (ht : Auth x.g m.p m.t) (hu : Auth x.g m.q m.t2) :
    Anchors (upOp c x {x.s with up := tail} m ok) := by
  have he := evolution_upOp c mutant x {x.s with up := tail} m ok (evolution_refl x)
  simp only [upOp,evolution_commit,evolution_ackMail] at he
  simp only [upOp]; split
  · apply anchors_commit
    apply anchors_transport x _ _ h hv
    · exact (evolution_refl x)
    · rfl
    · rfl
    · intro u hu; simp [hq,hu]
    · exact authMono_refl x.g
  · split <;> simp_all only [Bool.false_eq_true,ite_true,ite_false,evolution_commit,evolution_ackMail]
    · apply anchors_commit
      apply anchors_ackMail
      · apply anchors_ackMail
        · apply anchors_transport x _ _ h hv
          · exact (evolution_refl x)
          · rfl
          · rfl
          · intro u hu; simp [hq,hu]
          · exact authMono_refl x.g
        · simp
      · simp
    · apply anchors_commit
      apply anchors_ackMail
      · apply anchors_ackMail
        · apply anchors_transport x _ _ h hv
          · exact he
          · rfl
          · rfl
          · intro u hu; simp [hq,hu]
          · exact authMono_wrote x.g _ _
        · intro _ _
          dsimp only
          simp only [put,ite_true]
          by_cases hpq : m.q = m.p
          · simp only [hpq,ite_true,submitTo_takes c mutant]
            exact authMono_wrote x.g _ _ _ _ (by simpa [hpq] using ht)
          · simp only [hpq,ite_false,submitTo_takes c mutant]
            exact authMono_wrote x.g _ _ _ _ hu
      · intro _ _
        dsimp only [ackMail]
        split <;> simp only [put,ite_true,submitTo_takes c mutant]
        all_goals exact authMono_wrote x.g _ _ _ _ ht

theorem anchors_deliverUp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (hq : x.s.up = m::tail)
    (hc : CarrierAuth x) (hb : ByteDomain c x.s) : Anchors (deliverUp c x m tail ok) := by
  have mem : m ∈ x.s.up := by simp [hq]
  simp only [deliverUp]; split
  · exact anchors_upOpen c x m tail ok h hv hq
  · split
    · exact anchors_upClose c x m tail h hv hq
    · split
      · exact anchors_upDiscard c x m tail ok h hv hq
      · split
        · have hop : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
          exact anchors_upOp c mutant x m tail ok h hv hq
            (hc m mem (m.p,m.t) (by simp [carries,hop])) (hc m mem (m.q,m.t2) (by simp [carries,hop]))
        · have hkind : m.kind = "submit" := by
            have kinds := (hb.up m mem).1
            simp only [beq_iff_eq] at *
            simp_all
          exact anchors_upSubmit c mutant x m tail h hv hq (hc m mem (m.p,m.t) (by simp [carries,hkind]))
end Storage
