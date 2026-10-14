import CoreLineage21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def RealByte (c : Config P) (b : Int) : Prop := b = absent ∨ b ∈ c.texts
def RequestBytes (c : Config P) (m : Up P) : Prop :=
  m.kind ∈ ["open","close","discard","submit","op"] ∧
  (m.kind = "submit" ∨ m.kind = "op" → RealByte c m.t) ∧
  (m.kind = "op" → RealByte c m.t2)
structure ByteDomain (c : Config P) (s : Sys P) : Prop where
  disk : ∀ p, RealByte c (s.disk p)
  stable : ∀ p, RealByte c (s.stable p)
  pages : ∀ p, RealByte c (s.pages p).buf
  drafts : ∀ p, (s.drafts p).bytes = noneBytes ∨ RealByte c (s.drafts p).bytes
  job : s.job.on = true → RealByte c s.job.bytes
  w : ∀ p, RealByte c (s.w p).text
  mb : ∀ p, RealByte c (s.mb p).text
  up : ∀ m ∈ s.up, RequestBytes c m

@[simp] theorem real_absent (c : Config P) : RealByte c absent := Or.inl rfl

theorem bytes_init (c : Config P) : ByteDomain c (init c).s := by
  have hd : ∀ p : Fin P, RealByte c c.texts[p.val % c.texts.length]! := by
    intro p
    have hi : p.val % c.texts.length < c.texts.length := Nat.mod_lt _ (List.length_pos_iff.mpr c.nonempty)
    apply Or.inr
    rw [List.getElem!_eq_getElem?_getD,List.getElem?_eq_getElem hi]
    exact List.getElem_mem hi
  exact ⟨hd,hd,by simp [init],by simp [init],by simp [init,nojob],by simp [init],by simp [init],by simp [init]⟩

theorem bytes_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : ByteDomain c s) : ByteDomain c (commit c x s g n).s := by
  refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,h.w,?_,h.up⟩
  intro p
  change RealByte c (if _ then _ else s.mb p).text
  split
  · exact h.pages p
  · exact h.mb p

theorem bytes_ackMail (c : Config P) (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (h : ByteDomain c s) : ByteDomain c (ackMail s p ok v took) := by
  cases ok
  · exact h
  · refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,h.w,?_,h.up⟩
    intro q
    simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false]
    split
    · exact h.pages p
    · exact h.mb q

theorem table_real (c : Config P) (pg : Page) (d v : Int)
    (hp : RealByte c pg.buf) (hd : RealByte c d) : RealByte c (table c pg d v).buf := by
  simp only [table]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals first | exact hp | exact hd
theorem submit_real (c : Config P) (pg : Page) (t bv v : Int)
    (hp : RealByte c pg.buf) (ht : RealByte c t) : RealByte c (submitTo c pg t bv v).buf := by
  simp only [submitTo]; split <;> (try split)
  all_goals first | exact hp | exact ht
theorem reply_real (c : Config P) (pg : Page) (out : String) (b : Int)
    (hp : RealByte c pg.buf) : RealByte c (onReply c pg out b).buf := by
  simp only [onReply]; split <;> exact hp
theorem draft_real (c : Config P) (pg : Page) (hp : RealByte c pg.buf) :
    (draftEntry pg).bytes = noneBytes ∨ RealByte c (draftEntry pg).bytes := by
  simp only [draftEntry]; split
  · exact Or.inr hp
  · exact Or.inl rfl

theorem bytes_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ByteDomain c s) (hx : ByteDomain c x.s) : ByteDomain c (upOpen c x s m ok).s := by
  apply bytes_commit
  apply bytes_ackMail
  refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
  apply property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
  split
  · exact h.pages m.p
  · exact hx.disk m.p
theorem bytes_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : ByteDomain c s) : ByteDomain c (upClose c x s m).s := by
  apply bytes_commit
  split
  · refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
    exact property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages (real_absent c)
  · exact h
theorem bytes_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ByteDomain c s) (hx : ByteDomain c x.s) : ByteDomain c (upDiscard c x s m ok).s := by
  simp only [upDiscard]; split
  · exact bytes_commit c x _ _ _ h
  · split
    · exact bytes_commit c x _ _ _ (bytes_ackMail c _ _ _ _ _ h)
    · apply bytes_commit
      apply bytes_ackMail
      refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
      exact property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages (hx.disk m.p)
theorem bytes_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : ByteDomain c s) (ht : RealByte c m.t) : ByteDomain c (upSubmit c x s m).s := by
  simp only [upSubmit]; split
  · exact bytes_commit c x _ _ _ h
  · apply bytes_commit
    apply bytes_ackMail
    refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
    have hw : RealByte c (if m.ro != noneBytes && c.mutant != "MIS" then
        {s.pages m.p with buf := m.t, base := m.ro, typed := true, ver := x.g.vc + 1, conflict := (s.pages m.p).obs != m.ro, risk := (s.pages m.p).risk || (s.pages m.p).obs != m.ro}
        else submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 1)).buf := by
      split
      · exact ht
      · exact submit_real c _ _ _ _ (h.pages m.p) ht
    split
    · exact property_put _ _ _ (fun p : Page => RealByte c p.buf)
        (property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages hw) (real_absent c)
    · exact property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages hw
theorem bytes_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : ByteDomain c s) (ht : RealByte c m.t) (hu : RealByte c m.t2) : ByteDomain c (upOp c x s m ok).s := by
  simp only [upOp]; split
  · exact bytes_commit c x _ _ _ h
  · split
    · exact bytes_commit c x _ _ _ (bytes_ackMail c _ _ _ _ _ (bytes_ackMail c _ _ _ _ _ h))
    · apply bytes_commit
      apply bytes_ackMail
      apply bytes_ackMail
      refine ⟨h.disk,h.stable,?_,?_,h.job,h.w,h.mb,h.up⟩
      · exact property_put _ _ _ (fun p : Page => RealByte c p.buf)
          (property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
            (submit_real c _ _ _ _ (h.pages m.q) hu)) (submit_real c _ _ _ _ (h.pages m.p) ht)
      · split
        · exact h.drafts
        · exact property_put _ _ _ (fun d : Draft => d.bytes = noneBytes ∨ RealByte c d.bytes)
            h.drafts (draft_real c _ (submit_real c _ _ _ _ (h.pages m.q) hu))

theorem bytes_execute (c : Config P) (x : State P) (a : Action P)
    (h : ByteDomain c x.s) (hen : enabled c x a = true) : ByteDomain c (execute c x a).s := by
  cases a <;> simp only [execute]
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have hm := h.up m (by simp [hq])
      have htail : ByteDomain c {x.s with up := tail} :=
        ⟨h.disk,h.stable,h.pages,h.drafts,h.job,h.w,h.mb,fun u hu => h.up u (by simp [hq,hu])⟩
      simp only [deliverUp]; split
      · exact bytes_upOpen c x _ m ok htail h
      · split
        · exact bytes_upClose c x _ m htail
        · split
          · exact bytes_upDiscard c x _ m ok htail h
          · split
            · have hop : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
              exact bytes_upOp c x _ m ok htail (hm.2.1 (Or.inr hop)) (hm.2.2 hop)
            · have hkind : m.kind = "submit" := by
                simp only [RequestBytes,List.mem_cons,List.not_mem_nil,or_false] at hm
                simp only [beq_iff_eq] at *
                simp_all
              exact bytes_upSubmit c x _ m htail (hm.2.1 (Or.inl hkind))
  case wOpen p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text) h.w (real_absent c)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simp [RequestBytes,up1]
  case wEdit p v =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,h.pages,h.drafts,h.job,
      property_put _ _ _ (fun w : W => RealByte c w.text) h.w (Or.inr (by simpa using hen.2)),h.mb,h.up⟩
  case wSend p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text) h.w (h.w p)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simpa [RequestBytes,up1] using h.w p
  case wResolve p v =>
    have hv : RealByte c v := Or.inr (by simpa using hen.2)
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text) h.w hv
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simpa [RequestBytes,up1] using hv
  case wDiscard p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text) h.w (h.w p)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simp [RequestBytes,up1]
  case wClose p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text) h.w (real_absent c)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simp [RequestBytes,up1]
  case wOp p q t u =>
    have ht : RealByte c t := Or.inr (by simpa using hen.1.2)
    have hu : RealByte c u := Or.inr (by simpa using hen.2)
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => RealByte c w.text)
        (property_put _ _ _ (fun w : W => RealByte c w.text) h.w ht) hu
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · simpa [RequestBytes] using And.intro ht hu
  case wRecv p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,?_,?_,h.up⟩
    · apply property_put _ _ _ (fun w : W => RealByte c w.text) h.w
      split <;> (try split) <;> (try split) <;> (try split) <;> (try split)
      all_goals first | exact h.w p | exact h.mb p | exact real_absent c
    · exact property_put _ _ _ (fun m : Mail => RealByte c m.text) h.mb (real_absent c)
  case observe p =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
      (table_real c _ _ _ (h.pages p) (h.disk p)),h.drafts,h.job,h.w,h.mb,h.up⟩
  case flush p =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,h.pages,h.drafts,fun _ => h.pages p,h.w,h.mb,h.up⟩
  case check =>
    simp only [check]; split
    · apply bytes_commit
      exact ⟨h.disk,h.stable,property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
        (table_real c _ _ _ (h.pages _) (h.disk _)),h.drafts,by simp [nojob],h.w,h.mb,h.up⟩
    · apply bytes_commit
      exact ⟨h.disk,h.stable,h.pages,h.drafts,h.job,h.w,h.mb,h.up⟩
  case rename =>
    apply bytes_commit
    exact ⟨property_put _ _ _ (RealByte c) h.disk (h.job hen.1.2),h.stable,h.pages,h.drafts,h.job,h.w,h.mb,h.up⟩
  case dirSync ok =>
    apply bytes_commit
    refine ⟨h.disk,?_,?_,h.drafts,by simp [nojob],h.w,h.mb,h.up⟩
    · split
      · exact property_put _ _ _ (RealByte c) h.stable (h.disk _)
      · exact h.stable
    · apply property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
      split
      · exact h.pages _
      · exact reply_real c _ _ _ (h.pages _)
  case saveFail =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,property_put _ _ _ (fun p : Page => RealByte c p.buf) h.pages
      (reply_real c _ _ _ (h.pages _)),h.drafts,by simp [nojob],h.w,h.mb,h.up⟩
  case draftSync p =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,?_,h.job,h.w,h.mb,h.up⟩
    apply property_put _ _ _ (fun d : Draft => d.bytes = noneBytes ∨ RealByte c d.bytes) h.drafts
    split <;> exact draft_real c _ (h.pages p)
  case switchReq =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
    intro p; dsimp only; split <;> exact h.pages p
  case extWriteD p v dur =>
    have hv : RealByte c v := by simpa [RealByte] using hen.2
    apply bytes_commit
    refine ⟨property_put _ _ _ (RealByte c) h.disk hv,?_,h.pages,h.drafts,h.job,h.w,h.mb,h.up⟩
    split
    · exact property_put _ _ _ (RealByte c) h.stable hv
    · exact h.stable
  case windowCrash =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,h.pages,h.drafts,h.job,by simp,by simp,?_⟩
    split
    · simp
    · intro m hm; simp only [List.mem_map] at hm
      obtain ⟨u,hu,rfl⟩ := hm
      exact h.up u hu
  case launch =>
    apply bytes_commit
    refine ⟨h.disk,h.stable,?_,h.drafts,h.job,h.w,h.mb,h.up⟩
    intro p; simp only [restored]; split
    · exact real_absent c
    · rcases h.drafts p with hp | hp
      · simp_all
      · exact hp
  case switchFin =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,by simp [down],h.drafts,by simp [down,nojob],by simp [down],by simp [down],by simp [down]⟩
  case crash =>
    apply bytes_commit
    exact ⟨h.disk,h.stable,by simp [down],h.drafts,by simp [down,nojob],by simp [down],by simp [down],by simp [down]⟩
  case power keep =>
    have hd : ∀ p, RealByte c (if keep p then x.s.disk p else x.s.stable p) := by
      intro p; split
      · exact h.disk p
      · exact h.stable p
    apply bytes_commit
    exact ⟨hd,hd,by simp [down],h.drafts,by simp [down,nojob],by simp [down],by simp [down],by simp [down]⟩

theorem bytes_reachable (c : Config P) (x : State P) (hx : Reachable c x) : ByteDomain c x.s := by
  induction hx with
  | initial => exact bytes_init c
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact bytes_execute c _ a ih (by assumption)
    · contradiction
#print axioms bytes_reachable
end Storage21
