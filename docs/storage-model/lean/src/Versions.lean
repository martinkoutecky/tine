import Provenance
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

structure VersionBounds (s : Sys P) (v : Int) : Prop where
  nonnegative : 0 ≤ v
  pages : ∀ p, (s.pages p).ver ≤ v
  w : ∀ p, (s.w p).bv ≤ v
  mb : ∀ p, (s.mb p).ver ≤ v ∧ (s.mb p).ack ≤ v
  up : ∀ m ∈ s.up, m.bv ≤ v ∧ m.bv2 ≤ v

theorem bounds_mono (s : Sys P) (v u : Int) (h : VersionBounds s v) (hle : v ≤ u) : VersionBounds s u :=
  ⟨Int.le_trans h.nonnegative hle,fun p => Int.le_trans (h.pages p) hle,
    fun p => Int.le_trans (h.w p) hle,
    fun p => ⟨Int.le_trans (h.mb p).1 hle,Int.le_trans (h.mb p).2 hle⟩,
    fun m hm => ⟨Int.le_trans (h.up m hm).1 hle,Int.le_trans (h.up m hm).2 hle⟩⟩

theorem bounds_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : VersionBounds s g.vc) : VersionBounds (commit c x s g n).s (commit c x s g n).g.vc := by
  have hn := h.nonnegative
  refine ⟨h.nonnegative,h.pages,h.w,?_,h.up⟩
  intro p
  change (if _ then _ else s.mb p).ver ≤ g.vc ∧ (if _ then _ else s.mb p).ack ≤ g.vc
  split
  · constructor
    · exact h.pages p
    · split
      · exact (h.mb p).2
      · first | (dsimp only; omega) | omega
  · exact h.mb p

theorem bounds_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (u : Int) (h : VersionBounds s u) (hv : v ≤ u) : VersionBounds (ackMail s p ok v took) u := by
  cases ok
  · exact h
  · refine ⟨h.nonnegative,h.pages,h.w,?_,h.up⟩
    intro q
    simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false]
    split
    · exact ⟨h.pages p,hv⟩
    · exact h.mb q

theorem bounds_init (c : Config P) : VersionBounds (init c).s (init c).g.vc := by
  constructor <;> simp [init]

theorem table_ver (c : Config P) (pg : Page) (d v u : Int) (hp : pg.ver ≤ u) (hv : v ≤ u) :
    (table c pg d v).ver ≤ u := by
  simp only [table]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals first | exact hp | exact hv
theorem reply_ver (c : Config P) (pg : Page) (o : String) (b : Int) :
    (onReply c pg o b).ver = pg.ver := by simp only [onReply]; split <;> rfl
theorem submit_ver (c : Config P) (pg : Page) (t bv v u : Int) (hp : pg.ver ≤ u) (hv : v ≤ u) :
    (submitTo c pg t bv v).ver ≤ u := by
  simp only [submitTo]; split <;> (try split)
  all_goals first | exact hp | exact hv

theorem bounds_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : VersionBounds s x.g.vc) : VersionBounds (upOpen c x s m ok).s (upOpen c x s m ok).g.vc := by
  by_cases hb : ((s.pages m.p).held || !ok) = true
  · have hn : ¬ ((s.pages m.p).ver == x.g.vc+1) = true := by
      simp only [beq_iff_eq]; have hp := h.pages m.p; omega
    simp only [upOpen,hb,ite_true,hn,ite_false]
    apply bounds_commit
    apply bounds_ackMail _ _ _ _ _ _ _ (h.pages m.p)
    refine ⟨h.nonnegative,?_,h.w,h.mb,h.up⟩
    exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc) h.pages (h.pages m.p)
  · simp only [upOpen,hb,ite_false]
    simp only [Bool.false_eq_true,ite_false,beq_self_eq_true,ite_true]
    apply bounds_commit
    have hg := bounds_mono s x.g.vc (x.g.vc+1) h (by omega)
    apply bounds_ackMail _ _ _ _ _ _ _ (by first | (dsimp only; omega) | omega)
    refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
    exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages (by first | (dsimp only; omega) | omega)

theorem bounds_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : VersionBounds s x.g.vc) : VersionBounds (upClose c x s m).s (upClose c x s m).g.vc := by
  apply bounds_commit
  split
  · refine ⟨h.nonnegative,?_,h.w,h.mb,h.up⟩
    exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc) h.pages h.nonnegative
  · exact h

theorem bounds_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : VersionBounds s x.g.vc) : VersionBounds (upDiscard c x s m ok).s (upDiscard c x s m ok).g.vc := by
  simp only [upDiscard]; split
  · exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  · split
    · exact bounds_commit c x _ _ _ (bounds_ackMail _ _ _ _ _ _ h (h.pages m.p))
    · apply bounds_commit
      have hg := bounds_mono s x.g.vc (x.g.vc+1) h (by first | (dsimp only; omega) | omega)
      apply bounds_ackMail _ _ _ _ _ _ _ (by first | (dsimp only; omega) | omega)
      refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
      exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages (by first | (dsimp only; omega) | omega)

theorem bounds_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : VersionBounds s x.g.vc) : VersionBounds (upSubmit c x s m).s (upSubmit c x s m).g.vc := by
  simp only [upSubmit]; split
  · exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  · apply bounds_commit
    have hg := bounds_mono s x.g.vc (x.g.vc+1) h (by first | (dsimp only; omega) | omega)
    apply bounds_ackMail _ _ _ _ _ _ _ (by first | (dsimp only; omega) | omega)
    refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
    have hw : (if m.ro != noneBytes && c.mutant != "MIS" then {s.pages m.p with buf := m.t,base := m.ro,typed := true,ver := x.g.vc+1,conflict := (s.pages m.p).obs != m.ro,risk := (s.pages m.p).risk || (s.pages m.p).obs != m.ro} else submitTo c (s.pages m.p) m.t m.bv (x.g.vc+1)).ver ≤ x.g.vc+1 := by
      split
      · first | (dsimp only; omega) | omega
      · exact submit_ver c _ _ _ _ _ (hg.pages m.p) (by first | (dsimp only; omega) | omega)
    split
    · exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1)
        (property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages hw) hg.nonnegative
    · exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages hw

theorem bounds_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : VersionBounds s x.g.vc) : VersionBounds (upOp c x s m ok).s (upOp c x s m ok).g.vc := by
  simp only [upOp]; split
  · exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  · split
    · exact bounds_commit c x _ _ _ (bounds_ackMail _ _ _ _ _ _
        (bounds_ackMail _ _ _ _ _ _ h (h.pages m.q)) (h.pages m.p))
    · apply bounds_commit
      have hg := bounds_mono s x.g.vc (x.g.vc+2) h (by first | (dsimp only; omega) | omega)
      apply bounds_ackMail _ _ _ _ _ _ _ (by first | (dsimp only; omega) | omega)
      apply bounds_ackMail _ _ _ _ _ _ _ (by first | (dsimp only; omega) | omega)
      refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
      exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2)
        (property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2) hg.pages
          (submit_ver c _ _ _ _ _ (hg.pages m.q) (by first | (dsimp only; omega) | omega)))
        (submit_ver c _ _ _ _ _ (hg.pages m.p) (by first | (dsimp only; omega) | omega))

theorem restore_rank_lt (x : State P) (p : Fin P) (h : (x.s.drafts p).bytes ≠ noneBytes) :
    ((ids P).filter fun q => q.val < p.val && (x.s.drafts q).bytes != noneBytes).length <
    ((ids P).filter fun q => (x.s.drafts q).bytes != noneBytes).length := by
  have he : ((ids P).filter fun q => q.val < p.val && (x.s.drafts q).bytes != noneBytes) =
      (((ids P).filter fun q => (x.s.drafts q).bytes != noneBytes).filter fun q => q.val < p.val) := by
    simp only [List.filter_filter]
    try { congr 1; funext q; simp [Bool.and_comm] }
  rw [he]
  apply List.length_filter_lt_length_iff_exists.mpr
  exact ⟨p,by simp [ids,h],by simp⟩

theorem table_version_choice (c : Config P) (pg : Page) (d v : Int) :
    (table c pg d v).ver = pg.ver ∨ (table c pg d v).ver = v := by
  simp only [table]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals first | exact Or.inl rfl | exact Or.inr rfl

theorem bounds_table (c : Config P) (s : Sys P) (v : Int) (p : Fin P) (d : Int)
    (h : VersionBounds s v) :
    VersionBounds {s with pages := put s.pages p (table c (s.pages p) d (v+1))}
      (if (table c (s.pages p) d (v+1)).ver == v+1 then v+1 else v) := by
  split
  · have hg := bounds_mono s v (v+1) h (by omega)
    refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
    exact property_put _ _ _ (fun pg : Page => pg.ver ≤ v+1) hg.pages
      (table_ver c _ _ _ _ (hg.pages p) (by omega))
  · refine ⟨h.nonnegative,?_,h.w,h.mb,h.up⟩
    apply property_put _ _ _ (fun pg : Page => pg.ver ≤ v) h.pages
    rcases table_version_choice c (s.pages p) d (v+1) with hv | hv
    · rw [hv]; exact h.pages p
    · simp_all

theorem bounds_execute (c : Config P) (x : State P) (a : Action P)
    (h : VersionBounds x.s x.g.vc) : VersionBounds (execute c x a).s (execute c x a).g.vc := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have ht : VersionBounds {x.s with up := tail} x.g.vc :=
        ⟨h.nonnegative,h.pages,h.w,h.mb,fun u hu => h.up u (by simp [hq,hu])⟩
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact bounds_upOpen c x _ m ok ht
        | exact bounds_upClose c x _ m ht
        | exact bounds_upDiscard c x _ m ok ht
        | exact bounds_upOp c x _ m ok ht
        | exact bounds_upSubmit c x _ m ht
  case wOpen p =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (by have hn := h.nonnegative; dsimp only; omega)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · exact ⟨h.nonnegative,h.nonnegative⟩
  case wEdit p v =>
    apply bounds_commit
    exact ⟨h.nonnegative,h.pages,property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (h.w p),h.mb,h.up⟩
  case wSend p =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (h.w p),h.mb,?_⟩
    intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h.up m hm
    · exact ⟨h.w p,h.nonnegative⟩
  case wResolve p v =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (h.w p),h.mb,?_⟩
    intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h.up m hm
    · exact ⟨h.w p,h.nonnegative⟩
  case wDiscard p =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (h.w p),h.mb,?_⟩
    intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h.up m hm
    · exact ⟨h.w p,h.nonnegative⟩
  case wClose p =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (by have hn := h.nonnegative; dsimp only; omega)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · exact ⟨h.nonnegative,h.nonnegative⟩
  case wOp p q t u =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,?_,h.mb,?_⟩
    · exact property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc)
        (property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w (h.w p)) (h.w q)
    · intro m hm; simp only [List.mem_append,List.mem_singleton] at hm
      rcases hm with hm | rfl
      · exact h.up m hm
      · exact ⟨h.w p,h.w q⟩
  case wRecv p =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,?_,?_,h.up⟩
    · apply property_put _ _ _ (fun w : W => w.bv ≤ x.g.vc) h.w
      split <;> (try split) <;> (try split) <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact h.w p
        | exact (h.mb p).1
        | exact (h.mb p).2
        | (have hn := h.nonnegative; dsimp only; omega)
    · exact property_put _ _ _ (fun m : Mail => m.ver ≤ x.g.vc ∧ m.ack ≤ x.g.vc) h.mb
        (by have hn := h.nonnegative; constructor <;> dsimp only <;> omega)
  case observe p =>
    apply bounds_commit
    exact bounds_table c x.s x.g.vc p (x.s.disk p) h
  case flush p => exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  case check =>
    simp only [check]; split
    · apply bounds_commit
      exact bounds_table c {x.s with job := nojob c} x.g.vc x.s.job.p (x.s.disk x.s.job.p) ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
    · exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  case rename => exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  case dirSync ok =>
    apply bounds_commit
    refine ⟨h.nonnegative,?_,h.w,h.mb,h.up⟩
    apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc) h.pages
    split
    · exact h.pages _
    · simpa only [reply_ver] using h.pages x.s.job.p
  case saveFail =>
    apply bounds_commit
    exact ⟨h.nonnegative,property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc) h.pages
      (by simpa only [reply_ver] using h.pages x.s.job.p),h.w,h.mb,h.up⟩
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  case switchReq =>
    apply bounds_commit
    refine ⟨h.nonnegative,?_,h.w,h.mb,h.up⟩
    intro p; dsimp only; split <;> exact h.pages p
  case extWriteD p v dur => exact bounds_commit c x _ _ _ ⟨h.nonnegative,h.pages,h.w,h.mb,h.up⟩
  case windowCrash =>
    apply bounds_commit
    refine ⟨h.nonnegative,h.pages,?_,?_,?_⟩
    · intro p; have hn := h.nonnegative; dsimp only; omega
    · intro p; have hn := h.nonnegative; constructor <;> dsimp only <;> omega
    · split
      · simp
      · intro m hm; simp only [List.mem_map] at hm
        obtain ⟨u,hu,rfl⟩ := hm
        exact h.up u hu
  case launch =>
    apply bounds_commit
    have hg := bounds_mono x.s x.g.vc
      (x.g.vc+((ids P).filter fun p => (x.s.drafts p).bytes != noneBytes).length) h (by omega)
    refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
    intro p; simp only [restored]; split
    · have hn := hg.nonnegative; dsimp only; omega
    · have hd : (x.s.drafts p).bytes ≠ noneBytes := by simpa using (by assumption : ¬ ((x.s.drafts p).bytes == noneBytes) = true)
      have hr := restore_rank_lt x p hd
      dsimp only; omega
  case switchFin =>
    apply bounds_commit
    have hn := h.nonnegative
    constructor <;> simp [down,gDown] <;> omega
  case crash =>
    apply bounds_commit
    have hn := h.nonnegative
    constructor <;> simp [down,gDown] <;> omega
  case power keep =>
    apply bounds_commit
    have hn := h.nonnegative
    constructor <;> simp [down,gDown] <;> omega

theorem bounds_reachable (c : Config P) (x : State P) (hx : Reachable c x) : VersionBounds x.s x.g.vc := by
  induction hx with
  | initial => exact bounds_init c
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact bounds_execute c _ a ih
    · contradiction
#print axioms bounds_reachable
end Storage
