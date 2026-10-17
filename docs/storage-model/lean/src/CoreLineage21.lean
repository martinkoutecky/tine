import CoreTaken21
import CoreShape21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

abbrev EqualLineage (pg : Page) (pr : Promise) : Prop :=
  pr.bytes = pg.buf ∨ (pg.typed = false ∧ pg.buf = pg.base ∧ (pg.conflict = false → pg.buf = pg.obs))
def PromiseLineage (x : State P) : Prop :=
  (∀ p, (x.g.promise p).ver ≤ x.g.vc) ∧
  (∀ p, (x.s.pages p).held = true → (x.g.promise p).ver ≤ (x.s.pages p).ver ∧
    ((x.g.promise p).ver = (x.s.pages p).ver → EqualLineage (x.s.pages p) (x.g.promise p))) ∧ CleanSafe x

theorem ack_bound (pr : Promise) (b v : Int) (saved : Bool) (ep u : Int)
    (hp : pr.ver ≤ u) (hv : v ≤ u) : (ack pr b v saved ep).ver ≤ u := by
  simp only [ack]; split <;> first | exact hv | exact hp

theorem ack_lineage (pg : Page) (pr : Promise) (saved : Bool) (ep : Int)
    (hp : pr.ver ≤ pg.ver) (he : pr.ver = pg.ver → EqualLineage pg pr) :
    (ack pr pg.buf pg.ver saved ep).ver ≤ pg.ver ∧
    ((ack pr pg.buf pg.ver saved ep).ver = pg.ver → EqualLineage pg (ack pr pg.buf pg.ver saved ep)) := by
  simp only [ack]; split
  · exact ⟨Int.le_refl _,fun _ => Or.inl rfl⟩
  · exact ⟨hp,he⟩

theorem ack_safe (pg : Page) (pr : Promise) (saved : Bool) (ep : Int)
    (h : pg.buf = pg.obs ∨ (pr.ver ≥ pg.ver ∧ pr.bytes = pg.buf)) :
    pg.buf = pg.obs ∨ ((ack pr pg.buf pg.ver saved ep).ver ≥ pg.ver ∧ (ack pr pg.buf pg.ver saved ep).bytes = pg.buf) := by
  simp only [ack]; split
  · exact Or.inr ⟨Int.le_refl _,rfl⟩
  · exact h

theorem lineage_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : PromiseLineage ⟨s,g⟩) (hv : ∀ p, (s.pages p).ver ≤ g.vc) :
    PromiseLineage (commit c x s g n) := by
  refine ⟨?_,?_,?_⟩
  · intro p; change (PostPromises x s g p).ver ≤ g.vc
    simp only [PostPromises]; split
    · exact ack_bound _ _ _ _ _ _ (h.1 p) (hv p)
    · exact h.1 p
  · intro p hh
    change (PostPromises x s g p).ver ≤ (s.pages p).ver ∧
      ((PostPromises x s g p).ver = (s.pages p).ver → EqualLineage (s.pages p) (PostPromises x s g p))
    simp only [PostPromises]; split
    · exact ack_lineage _ _ _ _ (h.2.1 p hh).1 (h.2.1 p hh).2
    · exact h.2.1 p hh
  · intro p hc
    change (s.pages p).buf = (s.pages p).obs ∨
      ((PostPromises x s g p).ver ≥ (s.pages p).ver ∧ (PostPromises x s g p).bytes = (s.pages p).buf)
    simp only [PostPromises]; split
    · exact ack_safe _ _ _ _ (h.2.2 p hc)
    · exact h.2.2 p hc

theorem lineage_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : PromiseLineage ⟨s,g⟩) : PromiseLineage ⟨ackMail s p ok v t,g⟩ := by
  cases ok <;> simpa only [PromiseLineage,CleanSafe,pages_ackMail] using h

theorem lineage_fresh (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page)
    (h : PromiseLineage ⟨s,g⟩) (hv : g.vc < pg.ver)
    (hc : clean pg = true → pg.buf = pg.obs) :
    PromiseLineage ⟨{s with pages := put s.pages p pg},g⟩ := by
  refine ⟨h.1,?_,?_⟩
  · intro q hh; by_cases he : q = p
    · subst q
      have hp : (g.promise p).ver ≤ g.vc := h.1 p
      simp only [put,ite_true]
      exact ⟨by omega,by intro eq; omega⟩
    · simpa [put,he] using h.2.1 q (by simpa [put,he] using hh)
  · intro q cl; by_cases he : q = p
    · subst q; exact Or.inl (by simpa [put] using hc (by simpa [put] using cl))
    · simpa [put,he] using h.2.2 q (by simpa [put,he] using cl)

theorem equal_table (c : Config P) (mutant : c.mutant = "none") (pg : Page) (pr : Promise)
    (d v : Int) (h : EqualLineage pg pr) : EqualLineage (table c pg d v) pr ∨ (table c pg d v).ver = v := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  · left
    rcases h with h | ⟨ht,hbase,_⟩
    · exact Or.inl h
    · exact Or.inr ⟨ht,hbase,by intro _; simp_all⟩
  · right; rfl
  · left
    rcases h with h | ⟨ht,_,_⟩
    · exact Or.inl h
    · exact Or.inr ⟨ht,by simp_all,by intro _; simp_all⟩
  · left
    rcases h with h | ⟨ht,hbase,_⟩
    · exact Or.inl h
    · exact Or.inr ⟨ht,hbase,by simp⟩

theorem table_clean_observed (c : Config P) (mutant : c.mutant = "none") (pg : Page)
    (d v : Int) (hs : PageShape pg) (hc : clean (table c pg d v) = true) :
    (table c pg d v).buf = (table c pg d v).obs := by
  simp only [table,mutant] at hc ⊢
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false] at hc ⊢
  split at * <;> (try split at *) <;> (try split at *)
  all_goals simp_all [clean]
  all_goals grind

-- The page shape, rather than old observation equality, justifies retained clean reads.
theorem lineage_table (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (g : Ghost P) (p : Fin P) (d : Int)
    (h : PromiseLineage ⟨s,g⟩) (hs : Shapes s) (hv : VersionBounds s g.vc) :
    PromiseLineage ⟨{s with pages := put s.pages p (table c (s.pages p) d (g.vc+1))},
      {g with vc := if (table c (s.pages p) d (g.vc+1)).ver == g.vc+1 then g.vc+1 else g.vc}⟩ := by
  refine ⟨?_,?_,?_⟩
  · intro q; change (g.promise q).ver ≤ (if _ then g.vc+1 else g.vc)
    split
    · have hp : (g.promise q).ver ≤ g.vc := h.1 q
      exact Int.le_add_one hp
    · exact h.1 q
  · intro q hh
    by_cases he : q = p
    · subst q
      have held : (s.pages p).held = true := by simpa [put,table_held] using hh
      have old := h.2.1 p held
      simp only [put,ite_true]
      rcases table_version_choice c (s.pages p) d (g.vc+1) with eq | eq
      · refine ⟨by simpa [eq] using old.1,?_⟩
        intro peq
        rcases equal_table c mutant (s.pages p) (g.promise p) d (g.vc+1) (old.2 (by simpa [eq] using peq)) with same | fresh
        · exact same
        · have hp := hv.pages p; omega
      · have hp : (g.promise p).ver ≤ g.vc := h.1 p
        exact ⟨by simpa [eq] using (show (g.promise p).ver ≤ g.vc+1 by omega),by intro peq; rw [eq] at peq; omega⟩
    · simpa [put,he] using h.2.1 q (by simpa [put,he] using hh)
  · intro q hc; by_cases he : q = p
    · subst q
      have cl : clean (table c (s.pages p) d (g.vc+1)) = true := by simpa [put] using hc
      exact Or.inl (by simpa [put] using table_clean_observed c mutant _ d (g.vc+1) (hs p) cl)
    · simpa [put,he] using h.2.2 q (by simpa [put,he] using hc)

abbrev LineagePage (pg : Page) (pr : Promise) : Prop :=
  (pg.held = true → pr.ver ≤ pg.ver ∧ (pr.ver = pg.ver → EqualLineage pg pr)) ∧
  (clean pg = true → pg.buf = pg.obs ∨ (pr.ver ≥ pg.ver ∧ pr.bytes = pg.buf))

theorem lineage_update (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page) (pr : Promise) (v : Int)
    (h : PromiseLineage ⟨s,g⟩) (hv : g.vc ≤ v) (hb : pr.ver ≤ v) (hl : LineagePage pg pr) :
    PromiseLineage ⟨{s with pages := put s.pages p pg},{g with vc := v,promise := put g.promise p pr}⟩ := by
  refine ⟨?_,?_,?_⟩
  · intro q; by_cases he : q = p
    · subst q; simpa [put] using hb
    · have ho : (g.promise q).ver ≤ g.vc := h.1 q
      simpa [put,he] using Int.le_trans ho hv
  · intro q hh; by_cases he : q = p
    · subst q; simpa [put] using hl.1 (by simpa [put] using hh)
    · simpa [put,he] using h.2.1 q (by simpa [put,he] using hh)
  · intro q cl; by_cases he : q = p
    · subst q; simpa [put] using hl.2 (by simpa [put] using cl)
    · simpa [put,he] using h.2.2 q (by simpa [put,he] using cl)

theorem lineage_raise (s : Sys P) (g : Ghost P) (v : Int) (h : PromiseLineage ⟨s,g⟩)
    (hv : g.vc ≤ v) : PromiseLineage ⟨s,{g with vc := v}⟩ :=
  ⟨fun p => Int.le_trans (h.1 p) hv,h.2⟩

theorem fresh_lineagePage (pg : Page) (pr : Promise) (hp : pr.ver < pg.ver)
    (cl : clean pg = true → pg.buf = pg.obs) : LineagePage pg pr :=
  ⟨fun _ => ⟨Int.le_of_lt hp,by intro he; omega⟩,fun hc => Or.inl (cl hc)⟩

theorem line_ack_page (pg : Page) (pr : Promise) (sv : Bool) (ep : Int)
    (h : LineagePage pg pr) : LineagePage pg (ack pr pg.buf pg.ver sv ep) :=
  ⟨fun hh => ack_lineage _ _ _ _ (h.1 hh).1 (h.1 hh).2,
    fun hc => ack_safe _ _ _ _ (h.2 hc)⟩

@[simp] theorem put_same [DecidableEq α] (m : α → β) (p : α) : put m p (m p) = m := by
  funext q; by_cases he : q = p <;> simp_all [put]

theorem lineage_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PromiseLineage ⟨s,x.g⟩) (hv : VersionBounds s x.g.vc) : PromiseLineage (upOpen c x s m ok) := by
  by_cases go : ((s.pages m.p).held || !ok) = true
  · have nv : ¬ ((s.pages m.p).ver == x.g.vc+1) = true := by have hp := hv.pages m.p; simp only [beq_iff_eq]; omega
    simp only [upOpen,go,ite_true,nv,ite_false,Bool.false_eq_true]
    apply lineage_commit
    · apply lineage_ackMail
      simpa only [put_same] using h
    · intro p; simpa [pages_ackMail,put] using hv.pages p
  · simp only [upOpen,go,ite_false,Bool.false_eq_true,beq_self_eq_true,ite_true]
    apply lineage_commit
    · apply lineage_ackMail
      have fresh := lineage_fresh s x.g m.p {held := true,buf := x.s.disk m.p,base := x.s.disk m.p,obs := x.s.disk m.p,ver := x.g.vc+1} h (by (try dsimp only); omega) (by intro _; rfl)
      exact lineage_raise _ _ _ fresh (by omega)
    · intro p
      simp only [pages_ackMail,put]; split
      · (try dsimp only); omega
      · exact Int.le_add_one (hv.pages p)

theorem lineage_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : PromiseLineage ⟨s,x.g⟩) (hv : VersionBounds s x.g.vc) : PromiseLineage (upClose c x s m) := by
  simp only [upClose]; split
  · apply lineage_commit
    · simpa only [put_same] using lineage_update s x.g m.p {} (x.g.promise m.p) x.g.vc h (Int.le_refl _) (h.1 m.p)
        ⟨by simp,by intro _; exact Or.inl rfl⟩
    · intro p; simp only [put]; split
      · exact hv.nonnegative
      · exact hv.pages p
  · exact lineage_commit c x _ _ _ h hv.pages

theorem lineage_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : PromiseLineage ⟨s,x.g⟩) (hv : VersionBounds s x.g.vc) : PromiseLineage (upDiscard c x s m ok) := by
  simp only [upDiscard]; split
  · exact lineage_commit c x _ _ _ h hv.pages
  · split
    · exact lineage_commit c x _ _ _ (lineage_ackMail _ _ _ _ _ _ h) (by simpa only [pages_ackMail] using hv.pages)
    · apply lineage_commit
      · apply lineage_ackMail
        apply lineage_update s x.g
        · exact h
        · omega
        · split
          · have hp : (x.g.promise m.p).ver ≤ x.g.vc := h.1 m.p; (try dsimp only); omega
          · exact Int.le_refl _
        · constructor
          · intro hh
            split
            · have hp : (x.g.promise m.p).ver ≤ x.g.vc := h.1 m.p
              exact ⟨by (try dsimp only); omega,by intro eq; dsimp only at eq; omega⟩
            · exact ⟨Int.le_refl _,fun _ => Or.inr ⟨rfl,rfl,fun _ => rfl⟩⟩
          · intro _; exact Or.inl rfl
      · intro p; simp only [pages_ackMail,put]; split
        · (try dsimp only); omega
        · exact Int.le_add_one (hv.pages p)

theorem lineage_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : PromiseLineage ⟨s,x.g⟩)
    (hv : VersionBounds s x.g.vc) : PromiseLineage (upSubmit c x s m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  split
  · exact lineage_commit c x _ _ _ h hv.pages
  · apply lineage_commit
    · apply lineage_ackMail
      apply lineage_raise
      · apply lineage_fresh _ _ _ _ h
        · simp only [submitTo,mutant]
          simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
          split <;> (try split) <;> (try split) <;> dsimp only <;> omega
        · simp only [submitTo,mutant]
          simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
          split <;> (try split) <;> (try split) <;> simp [clean]
      · omega
    · intro p; simp only [pages_ackMail,put]; split
      · simp only [submitTo,mutant]
        simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
        split <;> (try split) <;> (try split) <;> dsimp only <;> omega
      · exact Int.le_add_one (hv.pages p)

@[simp] theorem submit_ver_exact (c : Config P) (mutant : c.mutant = "none") (pg : Page) (t bv v : Int) :
    (submitTo c pg t bv v).ver = v := by
  simp only [submitTo,mutant]
  simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
  split <;> rfl
@[simp] theorem submit_typed_exact (c : Config P) (mutant : c.mutant = "none") (pg : Page) (t bv v : Int) :
    (submitTo c pg t bv v).typed = true := by
  simp only [submitTo,mutant]
  simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
  split <;> rfl

theorem ack_newer (pr : Promise) (b v : Int) (sv : Bool) (ep : Int) (h : pr.ver < v) :
    ack pr b v sv ep = ⟨true,b,v,sv,ep⟩ := by simp [ack,h]

theorem lineage_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : PromiseLineage ⟨s,x.g⟩)
    (hv : VersionBounds s x.g.vc) : PromiseLineage (upOp c x s m ok) := by
  simp only [upOp]; split
  · exact lineage_commit c x _ _ _ h hv.pages
  · split
    · exact lineage_commit c x _ _ _ (lineage_ackMail _ _ _ _ _ _ (lineage_ackMail _ _ _ _ _ _ h))
        (by simpa only [pages_ackMail] using hv.pages)
    · have hp : (x.g.promise m.q).ver ≤ x.g.vc := h.1 m.q
      have ack := ack_newer (x.g.promise m.q) m.t2 (x.g.vc+1) false 0 (by omega)
      simp only [ack]
      apply lineage_commit
      · apply lineage_ackMail; apply lineage_ackMail
        refine ⟨?_,?_,?_⟩
        · intro p; simp only [put]; split
          · dsimp only; omega
          · have ho : (x.g.promise p).ver ≤ x.g.vc := h.1 p; (try dsimp only); omega
        · intro p hh
          by_cases hs : p = m.p
          · subst p
            simp only [put,ite_true,submit_ver_exact c mutant]
            split
            · dsimp only; exact ⟨by omega,by intro he; omega⟩
            · have ho : (x.g.promise m.p).ver ≤ x.g.vc := h.1 m.p
              exact ⟨by omega,by intro he; omega⟩
          · by_cases hd : p = m.q
            · subst p; simp only [put,hs,ite_false,ite_true,submit_ver_exact c mutant]
              exact ⟨Int.le_refl _,fun _ => Or.inl (submitTo_takes c mutant _ _ _ _).symm⟩
            · simpa [put,hs,hd] using h.2.1 p (by simpa [put,hs,hd] using hh)
        · intro p cl
          by_cases hs : p = m.p
          · subst p; simp [put,clean,submit_typed_exact c mutant] at cl
          · by_cases hd : p = m.q
            · subst p; simp [put,hs,clean] at cl
            · simpa [put,hs,hd] using h.2.2 p (by simpa [put,hs,hd] using cl)
      · intro p
        simp only [pages_ackMail,put]; split <;> (try split)
        all_goals first
          | (simp only [submit_ver_exact c mutant]; omega)
          | (have ho := hv.pages p; (try dsimp only); omega)

-- A live job keeps its page nonconflicted until completion.
def JobClear (s : Sys P) : Prop := s.job.on = true → (s.pages s.job.p).conflict = false

theorem jobClear_execute (c : Config P) (mutant : c.mutant = "none") (x : State P) (a : Action P)
    (h : JobClear x.s) (hj : JobSnapshot x.s) (hi : InactivePages x)
    (hen : enabled c x a = true) : JobClear (execute c x a).s := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      intro hon; rw [job_deliverUp] at hon ⊢
      have he : x.s.alive = true ∧ (!locked x.s m.p && (m.kind != "op" || !locked x.s m.q)) = true := by
        simpa only [enabled,hq,Bool.and_eq_true] using hen
      rw [snapshot_deliver_locked c mutant x m tail ok he.2 hon]
      exact h hon
  case observe p =>
    intro hon
    change x.s.job.on = true at hon
    have hp : x.s.job.p ≠ p := by simp [enabled,locked,hon] at hen; grind
    simpa [observe,put,hp] using h hon
  case flush p =>
    intro _; change (x.s.pages p).conflict = false
    simp [enabled] at hen; grind
  case check => simp only [check]; split
                · simp [JobClear,nojob,job_commit]
                · exact h
  case dirSync ok => simp [JobClear,dirSync,nojob,job_commit]
  case saveFail => simp [JobClear,saveFail,nojob,job_commit]
  case switchReq =>
    intro hon
    simp only [switchReq,pages_commit]; split <;> exact h hon
  case launch =>
    intro hon
    have dead : x.s.alive = false := by simpa [enabled] using hen
    have hh := (hj hon).1
    have hf := hi dead x.s.job.p
    simp_all
  case switchFin => simp [JobClear,switchFin,down,job_commit,nojob]
  case crash => simp [JobClear,crash,down,job_commit,nojob]
  case power keep => simp [JobClear,power,down,job_commit,nojob]
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> (try split) <;> exact h
  all_goals exact h

theorem jobClear_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : JobClear x.s := by
  induction hx with
  | initial => simp [JobClear,init,nojob]
  | @advance x y hr a hs ih =>
    have hi := inactive_reachable c x hr
    have hj := snapshot_reachable c mutant x hr
    unfold step at hs; split at hs
    · cases hs; exact jobClear_execute c mutant x a ih hj hi (by assumption)
    · contradiction

theorem line_reply_page (c : Config P) (mutant : c.mutant = "none") (pg : Page) (pr : Promise)
    (ok : Bool) (ep : Int) (hh : pg.held = true) (hc : pg.conflict = false)
    (h : LineagePage pg pr) :
    LineagePage (onReply c pg (if ok then "Published" else "Uncertain") pg.buf)
      (if ok then ack pr pg.buf pg.ver true ep else pr) := by
  cases ok
  · simpa [onReply,mutant,LineagePage,clean] using h.1
  · simp only [onReply,ite_true,beq_self_eq_true,Bool.true_or]
    constructor
    · intro _
      simp only [ack]; split
      · exact ⟨Int.le_refl _,fun _ => Or.inl rfl⟩
      · have ho := h.1 hh
        exact ⟨ho.1,fun eq => by
          rcases ho.2 eq with same | ⟨ht,hbase,hobs⟩
          · exact Or.inl same
          · exact Or.inr ⟨rfl,rfl,fun _ => hobs hc⟩⟩
    · intro _
      simp only [ack]; split
      · exact Or.inr ⟨Int.le_refl _,rfl⟩
      · have ho := h.1 hh
        have he : pr.ver = pg.ver := by
          have no : ¬ (pg.ver > pr.ver || pg.ver == pr.ver && pr.on && true && !pr.saved) = true := by assumption
          simp only [Bool.or_eq_true,decide_eq_true_eq] at no
          omega
        rcases ho.2 he with same | ⟨_,_,hobs⟩
        · exact Or.inr ⟨by simpa [he],same⟩
        · exact Or.inl (hobs hc)

theorem lineage_ack_at (s : Sys P) (g : Ghost P) (p : Fin P) (sv : Bool) (ep : Int)
    (h : PromiseLineage ⟨s,g⟩) (hv : VersionBounds s g.vc) (hh : (s.pages p).held = true) :
    PromiseLineage ⟨s,{g with promise := put g.promise p (ack (g.promise p) (s.pages p).buf (s.pages p).ver sv ep)}⟩ := by
  simpa only [put_same] using lineage_update s g p (s.pages p)
    (ack (g.promise p) (s.pages p).buf (s.pages p).ver sv ep) g.vc h (Int.le_refl _)
    (ack_bound _ _ _ _ _ _ (h.1 p) (hv.pages p))
    (line_ack_page _ _ _ _ ⟨h.2.1 p,h.2.2 p⟩)

theorem lineage_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : PromiseLineage x) (hs : Shapes x.s)
    (hv : VersionBounds x.s x.g.vc) (hj : JobSnapshot x.s) (hc : JobClear x.s)
    (hi : InactivePages x) (hen : enabled c x a = true) : PromiseLineage (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have ht : PromiseLineage ⟨{x.s with up := tail},x.g⟩ := h
      have bv : VersionBounds {x.s with up := tail} x.g.vc :=
        ⟨hv.nonnegative,hv.pages,hv.w,hv.mb,fun u hu => hv.up u (by simp [hq,hu])⟩
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact lineage_upOpen c x _ m ok ht bv
        | exact lineage_upClose c x _ m ht bv
        | exact lineage_upDiscard c x _ m ok ht bv
        | exact lineage_upOp c mutant x _ m ok ht bv
        | exact lineage_upSubmit c mutant x _ m ht bv
  case observe p =>
    exact lineage_commit c x _ _ _ (lineage_table c mutant x x.s x.g p _ h hs hv)
      (bounds_table c x.s x.g.vc p _ hv).pages
  case check =>
    simp only [check]; split
    · exact lineage_commit c x _ _ _ (lineage_table c mutant x {x.s with job := nojob c} x.g _ _ h hs ⟨hv.nonnegative,hv.pages,hv.w,hv.mb,hv.up⟩)
        (bounds_table c {x.s with job := nojob c} x.g.vc _ _ ⟨hv.nonnegative,hv.pages,hv.w,hv.mb,hv.up⟩).pages
    · exact lineage_commit c x _ _ _ h hv.pages
  case dirSync ok =>
    have hon : x.s.job.on = true := by simp [enabled] at hen; grind
    have snap := hj hon
    have clear := hc hon
    have prbound : (x.g.promise x.s.job.p).ver ≤ x.g.vc := h.1 _
    have localLine : LineagePage (x.s.pages x.s.job.p) (x.g.promise x.s.job.p) := ⟨h.2.1 _,h.2.2 _⟩
    simp only [dirSync,mutant]
    simp only [show ("none" == "MRE") = false from rfl,Bool.and_false,Bool.false_eq_true,ite_false]
    apply lineage_commit
    · cases ok
      · have localReply := line_reply_page c mutant _ _ false x.s.job.ep snap.1 clear localLine
        simpa only [PromiseLineage,CleanSafe,put_same,Bool.false_eq_true,ite_false,←snap.2.1,←snap.2.2.2.1] using
          lineage_update x.s x.g x.s.job.p
            (onReply c (x.s.pages x.s.job.p) "Uncertain" (x.s.pages x.s.job.p).buf)
            (x.g.promise x.s.job.p) x.g.vc h (Int.le_refl _) prbound localReply
      · have localReply := line_reply_page c mutant _ _ true x.s.job.ep snap.1 clear localLine
        simpa only [PromiseLineage,CleanSafe,ite_true,←snap.2.1,←snap.2.2.2.1] using
          lineage_update x.s x.g x.s.job.p
            (onReply c (x.s.pages x.s.job.p) "Published" (x.s.pages x.s.job.p).buf)
            (ack (x.g.promise x.s.job.p) (x.s.pages x.s.job.p).buf (x.s.pages x.s.job.p).ver true x.s.job.ep)
            x.g.vc h (Int.le_refl _) (ack_bound _ _ _ _ _ _ prbound (hv.pages _)) localReply
    · intro p; simp only [put]; split
      · simpa only [reply_ver] using hv.pages _
      · exact hv.pages p
  case saveFail =>
    apply lineage_commit
    · refine ⟨h.1,?_,?_⟩
      · intro p hh; by_cases he : p = x.s.job.p
        · subst p; simpa [put,onReply,mutant,EqualLineage] using h.2.1 x.s.job.p (by simpa [put,onReply,mutant] using hh)
        · simpa [put,he] using h.2.1 p (by simpa [put,he] using hh)
      · intro p cl; by_cases he : p = x.s.job.p
        · subst p; simp [put,onReply,mutant,clean] at cl
        · simpa [put,he] using h.2.2 p (by simpa [put,he] using cl)
    · intro p; simp only [put]; split
      · simpa only [reply_ver] using hv.pages _
      · exact hv.pages p
  case draftSync p =>
    simp only [draftSync,mutant]
    simp only [show ("none" == "MBD") = false from rfl,Bool.false_and,Bool.false_eq_true,ite_false]
    split
    · have hg : ((x.s.pages p).held && (x.s.pages p).risk) = true := by
        by_cases he : ((x.s.pages p).held && (x.s.pages p).risk) = true
        · exact he
        · simp [draftEntry,he] at *
      have held : (x.s.pages p).held = true := by
        have hf : (x.s.pages p).held = true ∧ (x.s.pages p).risk = true := by simpa using hg
        exact hf.1
      have ent : draftEntry (x.s.pages p) = ⟨(x.s.pages p).buf,(x.s.pages p).base,(x.s.pages p).ver⟩ := by simp [draftEntry,hg]
      simp only [ent]
      exact lineage_commit c x _ _ _ (lineage_ack_at x.s x.g p false 0 h hv held) hv.pages
    · exact lineage_commit c x _ _ _ h hv.pages
  case switchReq =>
    apply lineage_commit
    · refine ⟨h.1,?_,?_⟩
      · intro p hh; dsimp only at hh ⊢
        by_cases go : ((x.s.pages p).held && dirty (x.s.pages p)) = true
        · simp only [go,ite_true] at hh ⊢; exact h.2.1 p hh
        · simp only [go,Bool.false_eq_true,ite_false] at hh ⊢; exact h.2.1 p hh
      · intro p cl; dsimp only at cl ⊢
        by_cases go : ((x.s.pages p).held && dirty (x.s.pages p)) = true
        · simp [go,clean] at cl
        · simp only [go,ite_false] at cl ⊢; exact h.2.2 p cl
    · intro p; dsimp only; split <;> exact hv.pages p
  case launch =>
    apply lineage_commit
    · refine ⟨?_,?_,?_⟩
      · intro p; have hp : (x.g.promise p).ver ≤ x.g.vc := h.1 p; dsimp only; omega
      · intro p hh; simp only [restored] at hh ⊢
        by_cases go : ((x.s.drafts p).bytes == noneBytes) = true
        · simp [go] at hh
        · simp only [go,Bool.false_eq_true,ite_false] at hh ⊢
          have hp : (x.g.promise p).ver ≤ x.g.vc := h.1 p
          exact ⟨by (try dsimp only); omega,by intro eq; (try dsimp only at eq); omega⟩
      · intro p cl; simp only [restored] at cl ⊢
        by_cases go : ((x.s.drafts p).bytes == noneBytes) = true
        · simp [go]
        · simp [go,clean] at cl
    · exact (bounds_execute c x .launch hv).pages
  case power keep =>
    apply lineage_commit
    · refine ⟨?_,by simp [down],by simp [CleanSafe,down]⟩
      intro p; dsimp only; split <;> (try split) <;> (try split) <;> exact h.1 p
    · intro p; exact hv.nonnegative
  case switchFin => exact lineage_commit c x _ _ _ ⟨h.1,by simp [down],by simp [CleanSafe,down]⟩ (fun _ => hv.nonnegative)
  case crash => exact lineage_commit c x _ _ _ ⟨h.1,by simp [down],by simp [CleanSafe,down]⟩ (fun _ => hv.nonnegative)
  all_goals exact lineage_commit c x _ _ _ h hv.pages

theorem lineage_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : PromiseLineage x := by
  induction hx with
  | initial => simp [PromiseLineage,CleanSafe,EqualLineage,init]
  | @advance x y hr a hs ih =>
    have shape := shapes_reachable c mutant x hr
    have bounds := bounds_reachable c x hr
    have job := snapshot_reachable c mutant x hr
    have clear := jobClear_reachable c mutant x hr
    have inactive := inactive_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact lineage_execute c mutant x a ih shape bounds job clear inactive (by assumption)
    · contradiction

theorem cleanSafe_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : CleanSafe x := (lineage_reachable c mutant x hx).2.2
#print axioms lineage_reachable
#print axioms cleanSafe_reachable
end Storage21
