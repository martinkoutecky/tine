import CoreC21
import CoreBClean21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def SafeTexts (c : Config P) : Prop := ∀ t ∈ c.texts, t ≠ unknown
def TextSafe (s : Sys P) : Prop :=
  (∀ p, s.disk p ≠ unknown) ∧ (∀ p, s.stable p ≠ unknown) ∧
  (∀ p, (s.pages p).buf ≠ unknown) ∧ (∀ p, (s.drafts p).bytes ≠ unknown) ∧
  s.job.bytes ≠ unknown ∧ (∀ p, (s.w p).text ≠ unknown) ∧
  (∀ p, (s.mb p).text ≠ unknown) ∧ (∀ u ∈ s.up, u.t ≠ unknown ∧ u.t2 ≠ unknown)

theorem safe_init (c : Config P) (hs : SafeTexts c) : TextSafe (init c).s := by
  have hd : ∀ p : Fin P, c.texts[p.val % c.texts.length]! ≠ unknown := by
    intro p
    have hi : p.val % c.texts.length < c.texts.length := Nat.mod_lt _ (List.length_pos_iff.mpr c.nonempty)
    have hm : c.texts[p.val % c.texts.length]! ∈ c.texts := by
      rw [List.getElem!_eq_getElem?_getD, List.getElem?_eq_getElem hi]
      exact List.getElem_mem hi
    exact hs _ hm
  refine ⟨hd,hd,?_,?_,?_,?_,?_,?_⟩ <;> simp [init,nojob,absent,noneBytes,unknown]

theorem safe_put {m : Fin P → Int} {p : Fin P} {b : Int}
    (hm : ∀ q, m q ≠ unknown) (hb : b ≠ unknown) : ∀ q, put m p b q ≠ unknown := by
  intro q; unfold put; split <;> simp_all

theorem safe_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (hs : TextSafe s) : TextSafe (ackMail s p ok v took) := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  cases ok
  · exact ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  · refine ⟨hd,hst,hp,hdr,hj,hw,?_,hu⟩
    intro q; simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false]
    split <;> simp_all

theorem safe_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hs : TextSafe s) : TextSafe (commit c x s g n).s := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  refine ⟨hd,hst,hp,hdr,hj,hw,?_,hu⟩
  intro p
  change (if _ then _ else s.mb p).text ≠ unknown
  split <;> simp_all

theorem table_buf_safe (c : Config P) (pg : Page) (d v : Int)
    (hp : pg.buf ≠ unknown) (hd : d ≠ unknown) : (table c pg d v).buf ≠ unknown := by
  simp only [table]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals first | exact hp | exact hd

theorem submit_buf_safe (c : Config P) (pg : Page) (t bv v : Int)
    (hp : pg.buf ≠ unknown) (ht : t ≠ unknown) : (submitTo c pg t bv v).buf ≠ unknown := by
  unfold submitTo; split <;> (try split)
  all_goals first | exact hp | exact ht

theorem reply_buf_safe (c : Config P) (pg : Page) (out : String) (b : Int)
    (hp : pg.buf ≠ unknown) : (onReply c pg out b).buf ≠ unknown := by
  unfold onReply; split <;> exact hp

theorem draft_bytes_safe (pg : Page) (hp : pg.buf ≠ unknown) : (draftEntry pg).bytes ≠ unknown := by
  unfold draftEntry; split
  · exact hp
  · decide

theorem pages_put_safe (m : Fin P → Page) (p : Fin P) (pg : Page)
    (hm : ∀ q, (m q).buf ≠ unknown) (hb : pg.buf ≠ unknown) :
    ∀ q, (put m p pg q).buf ≠ unknown := property_put m p pg (fun pg : Page => pg.buf ≠ unknown) hm hb
theorem drafts_put_safe (m : Fin P → Draft) (p : Fin P) (dr : Draft)
    (hm : ∀ q, (m q).bytes ≠ unknown) (hb : dr.bytes ≠ unknown) :
    ∀ q, (put m p dr q).bytes ≠ unknown := property_put m p dr (fun dr : Draft => dr.bytes ≠ unknown) hm hb

theorem safe_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hs : TextSafe s) (hx : TextSafe x.s) : TextSafe (upOpen c x s m ok).s := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  apply safe_commit
  apply safe_ackMail
  refine ⟨hd,hst,?_,hdr,hj,hw,hm,hu⟩
  apply pages_put_safe _ _ _ hp
  split
  · exact hp m.p
  · exact hx.1 m.p

theorem safe_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (hs : TextSafe s) : TextSafe (upClose c x s m).s := by
  apply safe_commit
  split
  · rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
    exact ⟨hd,hst,pages_put_safe _ _ _ hp (by simp [up1,nojob,down,unknown,absent,noneBytes]),hdr,hj,hw,hm,hu⟩
  · exact hs

theorem safe_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hs : TextSafe s) (hx : TextSafe x.s) : TextSafe (upDiscard c x s m ok).s := by
  simp only [upDiscard]
  split
  · exact safe_commit c x _ _ _ hs
  · split
    · exact safe_commit c x _ _ _ (safe_ackMail _ _ _ _ _ hs)
    · apply safe_commit; apply safe_ackMail
      rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
      exact ⟨hd,hst,pages_put_safe _ _ _ hp (hx.1 m.p),hdr,hj,hw,hm,hu⟩

theorem safe_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (hs : TextSafe s) (ht : m.t ≠ unknown) : TextSafe (upSubmit c x s m).s := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  let pg := s.pages m.p
  have hn : (if m.ro != noneBytes && c.mutant != "MIS" then
      {pg with buf := m.t,base := m.ro,typed := true,ver := x.g.vc + 1, conflict := pg.obs != m.ro,risk := pg.risk || pg.obs != m.ro}
      else submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 1)).buf ≠ unknown := by
    split
    · exact ht
    · exact submit_buf_safe c _ _ _ _ (hp _) ht
  simp only [upSubmit]
  split
  · exact safe_commit c x _ _ _ ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  · apply safe_commit; apply safe_ackMail
    refine ⟨hd,hst,?_,hdr,hj,hw,hm,hu⟩
    split
    · apply pages_put_safe
      · exact pages_put_safe _ _ _ hp hn
      · decide
    · exact pages_put_safe _ _ _ hp hn

theorem safe_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hs : TextSafe s) (ht : m.t ≠ unknown) (ht2 : m.t2 ≠ unknown) : TextSafe (upOp c x s m ok).s := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  have ha := submit_buf_safe c (s.pages m.p) m.t m.bv (x.g.vc + 2) (hp _) ht
  have hb := submit_buf_safe c (s.pages m.q) m.t2 m.bv2 (x.g.vc + 1) (hp _) ht2
  simp only [upOp]
  split
  · exact safe_commit c x _ _ _ ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  · split
    · exact safe_commit c x _ _ _ (safe_ackMail _ _ _ _ _ (safe_ackMail _ _ _ _ _ ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩))
    · apply safe_commit; apply safe_ackMail; apply safe_ackMail
      refine ⟨hd,hst,?_,?_,hj,hw,hm,hu⟩
      · exact pages_put_safe _ _ _ (pages_put_safe _ _ _ hp hb) ha
      · split
        · exact hdr
        · apply drafts_put_safe _ _ _ hdr
          apply draft_bytes_safe
          exact hb

theorem safe_execute (c : Config P) (hstext : SafeTexts c) (x : State P) (a : Action P)
    (hs : TextSafe x.s) (hen : enabled c x a = true) : TextSafe (execute c x a).s := by
  rcases hs with ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  cases a <;> simp only [execute]
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
    | cons m tail =>
      have ht := hu m (by simp [hq])
      have htail : TextSafe {x.s with up := tail} :=
        ⟨hd,hst,hp,hdr,hj,hw,hm,fun u h => hu u (by simp [hq,h])⟩
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact safe_upOpen c x _ m ok htail ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
        | exact safe_upClose c x _ m htail
        | exact safe_upDiscard c x _ m ok htail ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
        | exact safe_upOp c x _ m ok htail ht.1 ht.2
        | exact safe_upSubmit c x _ m htail ht.1
  case wOpen p =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,?_,hm,?_⟩
    · apply property_put _ _ _ (fun w : W => w.text ≠ unknown) hw; simp [up1,nojob,down,unknown,absent,noneBytes]
    · intro u h; simp only [List.mem_append,List.mem_singleton] at h
      rcases h with h | h
      · exact hu u h
      · subst u; constructor <;> simp [up1,nojob,down,unknown,absent,noneBytes]
  case wEdit p v =>
    apply safe_commit
    exact ⟨hd,hst,hp,hdr,hj,property_put _ _ _ (fun w : W => w.text ≠ unknown) hw (hstext v (by simpa using hen.2)),hm,hu⟩
  case wSend p =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,property_put _ _ _ (fun w : W => w.text ≠ unknown) hw (hw p),hm,?_⟩
    intro u h; simp only [List.mem_append,List.mem_singleton] at h
    rcases h with h | h
    · exact hu u h
    · subst u; exact ⟨hw p,by simp [up1,nojob,down,unknown,absent,noneBytes]⟩
  case wResolve p v =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,property_put _ _ _ (fun w : W => w.text ≠ unknown) hw (hstext v (by simpa using hen.2)),hm,?_⟩
    intro u h; simp only [List.mem_append,List.mem_singleton] at h
    rcases h with h | h
    · exact hu u h
    · subst u; exact ⟨hstext v (by simpa using hen.2),by simp [up1,nojob,down,unknown,absent,noneBytes]⟩
  case wDiscard p =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,property_put _ _ _ (fun w : W => w.text ≠ unknown) hw (hw p),hm,?_⟩
    intro u h; simp only [List.mem_append,List.mem_singleton] at h
    rcases h with h | h
    · exact hu u h
    · subst u; constructor <;> simp [up1,nojob,down,unknown,absent,noneBytes]
  case wClose p =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,property_put _ _ _ (fun w : W => w.text ≠ unknown) hw (by simp [up1,nojob,down,unknown,absent,noneBytes]),hm,?_⟩
    intro u h; simp only [List.mem_append,List.mem_singleton] at h
    rcases h with h | h
    · exact hu u h
    · subst u; constructor <;> simp [up1,nojob,down,unknown,absent,noneBytes]
  case wOp p q t u =>
    have ht := hstext t (by simpa using hen.1.2)
    have ht2 := hstext u (by simpa using hen.2)
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,?_,hm,?_⟩
    · exact property_put _ _ _ (fun w : W => w.text ≠ unknown)
        (property_put _ _ _ (fun w : W => w.text ≠ unknown) hw ht) ht2
    · intro v h; simp only [List.mem_append,List.mem_singleton] at h
      rcases h with h | h
      · exact hu v h
      · subst v; exact ⟨ht,ht2⟩
  case wRecv p =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,?_,?_,hu⟩
    · apply property_put _ _ _ (fun w : W => w.text ≠ unknown) hw
      split <;> (try split) <;> (try split) <;> (try split) <;> (try split)
      all_goals first | exact hw p | exact hm p | simp [up1,nojob,down,unknown,absent,noneBytes]
    · exact property_put _ _ _ (fun m : Mail => m.text ≠ unknown) hm (by simp [up1,nojob,down,unknown,absent,noneBytes])
  case observe p =>
    apply safe_commit
    exact ⟨hd,hst,pages_put_safe _ _ _ hp (table_buf_safe c _ _ _ (hp p) (hd p)),hdr,hj,hw,hm,hu⟩
  case flush p =>
    apply safe_commit
    exact ⟨hd,hst,hp,hdr,hp p,hw,hm,hu⟩
  case check =>
    simp only [check]; split
    · apply safe_commit
      exact ⟨hd,hst,pages_put_safe _ _ _ hp (table_buf_safe c _ _ _ (hp _) (hd _)),hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],hw,hm,hu⟩
    · apply safe_commit
      exact ⟨hd,hst,hp,hdr,hj,hw,hm,hu⟩
  case rename =>
    apply safe_commit
    exact ⟨safe_put hd hj,hst,hp,hdr,hj,hw,hm,hu⟩
  case dirSync ok =>
    apply safe_commit
    refine ⟨hd,?_,?_,hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],hw,hm,hu⟩
    · split
      · exact safe_put hst (hd _)
      · exact hst
    · apply pages_put_safe _ _ _ hp
      split
      · exact hp _
      · exact reply_buf_safe c _ _ _ (hp _)
  case saveFail =>
    apply safe_commit
    exact ⟨hd,hst,pages_put_safe _ _ _ hp (reply_buf_safe c _ _ _ (hp _)),hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],hw,hm,hu⟩
  case draftSync p =>
    apply safe_commit
    refine ⟨hd,hst,hp,?_,hj,hw,hm,hu⟩
    apply drafts_put_safe _ _ _ hdr
    split <;> exact draft_bytes_safe _ (hp p)
  case switchReq =>
    apply safe_commit
    refine ⟨hd,hst,?_,hdr,hj,hw,hm,hu⟩
    intro p; dsimp only; split <;> exact hp p
  case extWriteD p v dur =>
    have hv : v ≠ unknown := by
      rcases hen.2 with h | h
      · subst v; simp [up1,nojob,down,unknown,absent,noneBytes]
      · exact hstext v (by simpa using h)
    apply safe_commit
    refine ⟨safe_put hd hv,?_,hp,hdr,hj,hw,hm,hu⟩
    split
    · exact safe_put hst hv
    · exact hst
  case windowCrash =>
    apply safe_commit
    refine ⟨hd,hst,hp,hdr,hj,by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],?_⟩
    split
    · simp
    · intro u h; simp only [List.mem_map] at h
      obtain ⟨v,hv,rfl⟩ := h
      exact hu v hv
  case launch =>
    apply safe_commit
    refine ⟨hd,hst,?_,hdr,hj,hw,hm,hu⟩
    intro p; simp only [restored]; split
    · decide
    · exact hdr p
  case switchFin =>
    apply safe_commit
    exact ⟨hd,hst,by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by simp [down]⟩
  case crash =>
    apply safe_commit
    exact ⟨hd,hst,by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by simp [down]⟩
  case power keep =>
    apply safe_commit
    have hnd : ∀ p, (if keep p then x.s.disk p else x.s.stable p) ≠ unknown := by
      intro p; split
      · exact hd p
      · exact hst p
    exact ⟨hnd,hnd,by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],hdr,by simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by intro _; simp [up1,nojob,down,unknown,absent,noneBytes],by simp [down]⟩

theorem safe_step (c : Config P) (ht : SafeTexts c) (x y : State P) (a : Action P)
    (hx : TextSafe x.s) (hs : step c x a = some y) : TextSafe y.s := by
  unfold step at hs
  split at hs
  · cases hs; exact safe_execute c ht x a hx (by assumption)
  · contradiction

theorem textSafe_reachable (c : Config P) (ht : SafeTexts c) (x : State P)
    (hx : Reachable c x) : TextSafe x.s := by
  induction hx with
  | initial => exact safe_init c ht
  | advance hr a hs ih => exact safe_step c ht _ _ a ih hs

theorem disk_not_unknown_reachable (c : Config P) (ht : SafeTexts c) (x : State P)
    (hx : Reachable c x) (p : Fin P) : x.s.disk p ≠ unknown :=
  (textSafe_reachable c ht x hx).1 p

def positiveTexts (T : Nat) : List Int := (List.range T).map fun n => (n + 1 : Nat)
theorem positiveTexts_safe (c : Config P) (T : Nat) (ht : c.texts = positiveTexts T) : SafeTexts c := by
  intro t hm
  rw [ht] at hm
  obtain ⟨n,_,rfl⟩ := List.mem_map.mp hm
  simp only [unknown]
  omega

theorem positiveTexts_nonempty (T : Nat) (hT : 0 < T) : positiveTexts T ≠ [] := by
  simp [positiveTexts]; omega

theorem positiveTexts_disk_not_unknown (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts T) (x : State P) (hx : Reachable c x) (p : Fin P) :
    x.s.disk p ≠ unknown := disk_not_unknown_reachable c (positiveTexts_safe c T texts) x hx p

#print axioms safe_init
#print axioms textSafe_reachable
#print axioms disk_not_unknown_reachable
#print axioms positiveTexts_disk_not_unknown
end Storage21
