import PairFrames11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false

theorem upOpen_pagesSafe11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) :
    ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p)
      ((Storage21.upOpen c x {x.s with up := tail} m ok).s.pages p) := by
  simp only [Storage21.upOpen,Storage21.pages_commit,Storage21.pages_ackMail]
  apply pageSafe_put11
  · exact fun p => pageSafe_refl11 _ _
  · exact pageSafe_open11 _ _ _ _

theorem upClose_pagesSafe11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (mutant : c.mutant = "none") :
    ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p)
      ((Storage21.upClose c x {x.s with up := tail} m).s.pages p) := by
  have hp := pageSafe_close11 c (x.s.pages m.p) (x.s.disk m.p) mutant
  simp only [Storage21.upClose,Storage21.pages_commit]
  split
  · apply pageSafe_put11
    · exact fun p => pageSafe_refl11 _ _
    · have cond : ((x.s.pages m.p).held && (clean (x.s.pages m.p) || (c.mutant == "MCL" && !(x.s.pages m.p).risk))) = true := by assumption
      simpa only [cond,ite_true] using hp
  · exact fun p => pageSafe_refl11 _ _

theorem upSubmit_pagesSafe11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (mutant : c.mutant = "none") (text : m.t ≠ absent) :
    ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p)
      ((Storage21.upSubmit c x {x.s with up := tail} m).s.pages p) := by
  simp only [Storage21.upSubmit]
  split
  · exact fun p => pageSafe_refl11 _ _
  · simp only [Storage21.pages_commit,Storage21.pages_ackMail,mutant,
      show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
    apply pageSafe_put11
    · exact fun p => pageSafe_refl11 _ _
    · split
      · exact pageSafe_resolve11 _ _ _ _ _ text
      · exact pageSafe_submit11 c _ _ _ _ _ mutant text

theorem upOp_pagesSafe11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) (mutant : c.mutant = "none")
    (text : m.t ≠ absent) (text2 : m.t2 ≠ absent) :
    ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p)
      ((Storage21.upOp c x {x.s with up := tail} m ok).s.pages p) := by
  simp only [Storage21.upOp]
  split <;> (try split)
  · exact fun p => pageSafe_refl11 _ _
  · simpa only [Storage21.pages_commit,Storage21.pages_ackMail] using
      (show ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p) (x.s.pages p) from fun p => pageSafe_refl11 _ _)
  · simp only [Storage21.pages_commit,Storage21.pages_ackMail]
    apply pageSafe_put11
    · apply pageSafe_put11
      · exact fun p => pageSafe_refl11 _ _
      · exact pageSafe_raise11 _ _ _ (pageSafe_submit11 c _ _ _ _ _ mutant text2)
    · exact pageSafe_submit11 c _ _ _ _ _ mutant text

theorem delivery_pagesSafe11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) (mutant : c.mutant = "none")
    (kind : m.kind ∈ ["open","close","discard","submit","op"])
    (texts : RequestText11 m) (nodiscard : m.kind ≠ "discard") :
    ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p)
      ((Storage21.deliverUp c x m tail ok).s.pages p) := by
  simp only [Storage21.deliverUp]
  split
  · exact upOpen_pagesSafe11 c x m tail ok
  · split
    · exact upClose_pagesSafe11 c x m tail mutant
    · split
      · exact False.elim (nodiscard (by simpa using (by assumption : (m.kind == "discard") = true)))
      · split
        · have op : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
          exact upOp_pagesSafe11 c x m tail ok mutant (texts.2 op).1 (texts.2 op).2
        · have submit : m.kind = "submit" := by simp_all
          exact upSubmit_pagesSafe11 c x m tail mutant (texts.1 submit)

theorem delivery_disk11 (c : Config P) (x : Storage21.State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) : (Storage21.deliverUp c x m tail ok).s.disk = x.s.disk := by
  simp only [Storage21.deliverUp]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upOp,Storage21.upSubmit]
  all_goals repeat (first | rfl | split | (cases m.cur <;> rfl))

theorem pairSafety_core_delivery11 (c : Config P) (x : State P) (m : Up P)
    (tail : List (Up P)) (ok : Bool) (mutant : c.mutant = "none") (h : PairSafety10 x)
    (kind : m.kind ∈ ["open","close","discard","submit","op"])
    (texts : RequestText11 m) (nodiscard : m.kind ≠ "discard") :
    PairSafety10 (coreLift11 x (Storage21.deliverUp c (core11 x) m tail ok)) := by
  apply pairSafety_core11 x _ h
  · exact delivery_disk11 c (core11 x) m tail ok
  · exact delivery_pagesSafe11 c (core11 x) m tail ok mutant kind texts nodiscard
  · rw [Storage21.job_delivery9 c (core11 x) m tail ok]
    exact fun on => ⟨on,rfl,rfl⟩
end Storage35
