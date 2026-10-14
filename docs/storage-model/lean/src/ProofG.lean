import ProofC
import Domain
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

def GTag : String := "G-overwrote-unseen"
def Auth (g : Ghost P) (p : Fin P) (b : Int) : Prop :=
  b ∈ g.seen p ∨ ∃ v, (b,v) ∈ g.wrote p
def Admissible (g : Ghost P) (p : Fin P) (bytes base : Int) : Prop :=
  base = unknown ∨ base = bytes ∨ Auth g p base
def PageBases (x : State P) : Prop :=
  ∀ p, (x.s.pages p).held = true →
    Admissible x.g p (x.s.pages p).buf (x.s.pages p).base
def DraftBases (x : State P) : Prop :=
  ∀ p, (x.s.drafts p).bytes ≠ noneBytes →
    Admissible x.g p (x.s.drafts p).bytes (x.s.drafts p).base
def JobBase (x : State P) : Prop :=
  x.s.job.on = true → Admissible x.g x.s.job.p x.s.job.bytes x.s.job.base
def JobWritten (x : State P) : Prop :=
  x.s.job.on = true → x.s.job.phase = 3 → Auth x.g x.s.job.p x.s.job.bytes
def CheckedReal (x : State P) : Prop :=
  x.s.job.on = true → x.s.job.phase = 2 → x.g.guard = true → x.s.job.base ≠ unknown
def GInv (x : State P) : Prop :=
  GTag ∉ x.g.bad ∧ PageBases x ∧ DraftBases x ∧ JobBase x ∧ JobWritten x ∧ CheckedReal x
def AuthMono (g h : Ghost P) : Prop := ∀ p b, Auth g p b → Auth h p b
attribute [reducible] Auth Admissible AuthMono PageBases DraftBases JobBase JobWritten CheckedReal

theorem mem_union_general [BEq α] [LawfulBEq α] (a b : List α) (t : α) :
    t ∈ union a b ↔ t ∈ a ∨ t ∈ b := by
  induction b generalizing a with
  | nil => simp [union]
  | cons v vs ih =>
    simp only [union, List.foldl_cons]
    change t ∈ union (if a.contains v then a else a ++ [v]) vs ↔ _
    rw [ih]
    split <;> simp_all <;> grind

theorem Auth_mono_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    AuthMono g (commit c x s g n).g := by
  intro p b h
  rcases h with h | ⟨v,h⟩
  · exact Or.inl ((mem_union_general _ _ _).mpr (Or.inl h))
  · exact Or.inr ⟨v,h⟩

theorem admissible_mono (hm : AuthMono g k) (ha : Admissible g p b a) :
    Admissible k p b a := by
  rcases ha with h | h | h
  · exact Or.inl h
  · exact Or.inr (Or.inl h)
  · exact Or.inr (Or.inr (hm p a h))

@[simp] theorem G_bad_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    GTag ∉ (commit c x s g n).g.bad ↔ GTag ∉ g.bad := by simp [commit,GTag]

theorem G_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : GInv ⟨s,g⟩) : GInv (commit c x s g n) := by
  rcases h with ⟨hb,hp,hd,hj,hw,hc⟩
  refine ⟨(G_bad_commit c x s g n).mpr hb,?_,?_,?_,?_,?_⟩
  · intro p hh
    exact admissible_mono (Auth_mono_commit c x s g n) (hp p hh)
  · intro p hh
    exact admissible_mono (Auth_mono_commit c x s g n) (hd p hh)
  · intro hh
    exact admissible_mono (Auth_mono_commit c x s g n) (hj hh)
  · intro hh ph
    exact Auth_mono_commit c x s g n _ _ (hw hh ph)
  · exact hc

theorem G_init (c : Config P) : GInv (init c) := by
  simp [GInv,PageBases,DraftBases,JobBase,JobWritten,CheckedReal,init,nojob,noneBytes]

theorem G_frame (x y : State P) (hx : GInv x)
    (hp : y.s.pages = x.s.pages) (hd : y.s.drafts = x.s.drafts)
    (hj : y.s.job = x.s.job) (hg : y.g.guard = x.g.guard)
    (hm : AuthMono x.g y.g) (hb : GTag ∉ y.g.bad) : GInv y := by
  rcases hx with ⟨_,hpg,hdr,hjb,hwr,hcr⟩
  refine ⟨hb,?_,?_,?_,?_,?_⟩
  · intro p hh; apply admissible_mono hm; simpa only [hp] using hpg p (by simpa [hp] using hh)
  · intro p hh; apply admissible_mono hm; simpa only [hd] using hdr p (by simpa [hd] using hh)
  · intro hh; simpa only [hj] using admissible_mono hm (hjb (by simpa [hj] using hh))
  · intro hh ph; simpa only [hj] using hm _ _ (hwr (by simpa [hj] using hh) (by simpa [hj] using ph))
  · simpa only [CheckedReal,hj,hg] using hcr

theorem auth_seen_wrote (x : State P) (p : Fin P) (b : Int) :
    Auth x.g p b ↔ ((x.g.seen p).contains b || (x.g.wrote p).any fun w => w.1 == b) = true := by
  simp [Auth,List.any_eq_true]

theorem G_rename_safe (x : State P) (hx : GInv x)
    (hon : x.s.job.on = true) (hphase : x.s.job.phase = 2) :
    (x.g.guard && x.s.job.bytes != x.s.job.base &&
      !((x.g.seen x.s.job.p).contains x.s.job.base ||
        (x.g.wrote x.s.job.p).any fun w => w.1 == x.s.job.base)) = false := by
  rcases hx with ⟨_,_,_,hj,_,hc⟩
  by_cases hg : x.g.guard = true
  · have hn := hc hon hphase hg
    rcases hj hon with h | h | h
    · exact False.elim (hn h)
    · simp [h]
    · rcases h with h | ⟨v,h⟩
      · simp [hg,h]
      · have hw : (x.g.wrote x.s.job.p).any (fun w => w.1 == x.s.job.base) = true :=
          List.any_eq_true.mpr ⟨(x.s.job.base,v),h,by simp⟩
        simp only [hw,Bool.or_true,Bool.not_true,Bool.and_false]
  · have hg : x.g.guard = false := by cases h : x.g.guard <;> simp_all
    simp [hg]

theorem authMono_refl (g : Ghost P) : AuthMono g g := fun _ _ h => h

theorem authMono_seen (g : Ghost P) (p : Fin P) (bs : List Int) :
    AuthMono g {g with seen := put g.seen p (union (g.seen p) bs)} := by
  intro q b h
  rcases h with h | ⟨v,h⟩
  · apply Or.inl
    by_cases hq : q = p
    · simp only [put,hq,ite_true]; exact (mem_union_general _ _ _).mpr (Or.inl (by simpa [hq] using h))
    · simpa [put,hq] using h
  · exact Or.inr ⟨v,h⟩

theorem authMono_wrote (g : Ghost P) (p : Fin P) (ws : List (Int × Int)) :
    AuthMono g {g with wrote := put g.wrote p (union (g.wrote p) ws)} := by
  intro q b h
  rcases h with h | ⟨v,h⟩
  · exact Or.inl h
  · apply Or.inr; refine ⟨v,?_⟩
    by_cases hq : q = p
    · simp only [put,hq,ite_true]; exact (mem_union_general _ _ _).mpr (Or.inl (by simpa [hq] using h))
    · simpa [put,hq] using h

theorem auth_wrote_added (g : Ghost P) (p : Fin P) (b v : Int) :
    Auth {g with wrote := put g.wrote p (union (g.wrote p) [(b,v)])} p b := by
  apply Or.inr; refine ⟨v,?_⟩
  simp only [put,ite_true]
  exact (mem_union_general _ _ _).mpr (Or.inr (by simp))

theorem admissible_table (c : Config P) (mutant : c.mutant = "none")
    (g : Ghost P) (p : Fin P) (pg : Page) (d v : Int)
    (h : pg.held = true → Admissible g p pg.buf pg.base) :
    (table c pg d v).held = true →
    Admissible g p (table c pg d v).buf (table c pg d v).base := by
  simp only [table,mutant]; simp only [show ("none" == "MQ") = false from rfl,
    show ("none" == "MLO") = false from rfl,Bool.false_and,Bool.or_false,
    Bool.false_eq_true,ite_false]
  split
  · exact h
  · split
    · intro _; exact Or.inr (Or.inl rfl)
    · split
      · intro _; apply Or.inr; apply Or.inl; simp_all
      · exact h

theorem pageBases_put (x : State P) (p : Fin P) (pg : Page) (hp : PageBases x)
    (h : pg.held = true → Admissible x.g p pg.buf pg.base) :
    PageBases ⟨{x.s with pages := put x.s.pages p pg},x.g⟩ := by
  intro q hh
  by_cases he : q = p
  · subst q; simpa [put] using h (by simpa [put] using hh)
  · simpa [put,he] using hp q (by simpa [put,he] using hh)

theorem G_observe (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (p : Fin P) (hx : GInv x) : GInv (observe c x p) := by
  apply G_commit
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  refine ⟨hb,?_,hd,hj,hw,hc⟩
  exact pageBases_put x p _ hp (admissible_table c mutant x.g p _ _ _ (hp p))

theorem G_flush (c : Config P) (x : State P) (p : Fin P) (hx : GInv x)
    (held : (x.s.pages p).held = true) : GInv (flush c x p) := by
  apply G_commit
  rcases hx with ⟨hb,hp,hd,_,_,_⟩
  refine ⟨hb,hp,hd,?_,?_,?_⟩
  · intro _; exact hp p held
  · simp [JobWritten]
  · simp [CheckedReal]

theorem G_check (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : GInv x) (phase : x.s.job.phase = 1)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) : GInv (check c x) := by
  rcases hx with ⟨hb,hp,hd,hj,_,_⟩
  simp only [check,mutant]
  split
  · apply G_commit
    refine ⟨hb,?_,hd,?_,?_,?_⟩
    · exact pageBases_put x _ _ hp (admissible_table c mutant x.g _ _ _ _ (hp _))
    · simp [JobBase,nojob]
    · simp [JobWritten,nojob]
    · simp [CheckedReal,nojob]
  · apply G_commit
    refine ⟨hb,hp,hd,hj,?_,?_⟩
    · simp [JobWritten]
    · intro _ _ hg
      change (x.s.disk x.s.job.p == x.s.job.base) = true at hg
      have he : x.s.disk x.s.job.p = x.s.job.base := by simpa using hg
      simpa [← he] using diskReal

theorem G_rename (c : Config P) (x : State P) (hx : GInv x)
    (hon : x.s.job.on = true) (hp : x.s.job.phase = 2) : GInv (rename c x) := by
  have safe := G_rename_safe x hx hon hp
  rcases hx with ⟨hb,hpg,hdr,hjb,_,_⟩
  apply G_commit
  have hm := authMono_wrote x.g x.s.job.p [(x.s.job.bytes,x.s.job.ver)]
  have added := auth_wrote_added x.g x.s.job.p x.s.job.bytes x.s.job.ver
  refine ⟨?_,?_,?_,?_,?_,?_⟩
  · change GTag ∉ union (union x.g.bad (tags _ "C-overwrote-external")) (tags _ GTag)
    rw [safe]
    simpa [GTag] using hb
  · intro p h; exact admissible_mono hm (hpg p h)
  · intro p h; exact admissible_mono hm (hdr p h)
  · intro h; exact admissible_mono hm (hjb h)
  · intro _ _; exact added
  · simp [CheckedReal]

theorem admissible_reply (c : Config P) (mutant : c.mutant = "none")
    (g : Ghost P) (p : Fin P) (pg : Page) (out : String) (b : Int)
    (ha : pg.held = true → Admissible g p pg.buf pg.base)
    (hb : out = "Published" → Auth g p b) :
    (onReply c pg out b).held = true →
    Admissible g p (onReply c pg out b).buf (onReply c pg out b).base := by
  simp only [onReply,mutant]
  simp only [show ("none" == "MLS") = false from rfl,Bool.and_false,Bool.or_false]
  split
  · intro _; exact Or.inr (Or.inr (hb (by simpa using (by assumption : (out == "Published") = true))))
  · exact ha

theorem G_dirSync (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (ok : Bool) (hx : GInv x)
    (hon : x.s.job.on = true) (phase : x.s.job.phase = 3) : GInv (dirSync c x ok) := by
  rcases hx with ⟨hb,hp,hd,_,hw,_⟩
  unfold dirSync
  simp only [mutant,show ("none" == "MRE") = false from rfl,Bool.and_false,ite_false]
  apply G_commit
  refine ⟨hb,?_,hd,?_,?_,?_⟩
  · exact pageBases_put x _ _ hp (admissible_reply c mutant x.g _ _ _ _ (hp _) (fun _ => hw hon phase))
  · simp [JobBase,nojob]
  · simp [JobWritten,nojob]
  · simp [CheckedReal,nojob]

theorem G_saveFail (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : GInv x) : GInv (saveFail c x) := by
  rcases hx with ⟨hb,hp,hd,_,_,_⟩
  apply G_commit
  refine ⟨hb,?_,hd,?_,?_,?_⟩
  · exact pageBases_put x _ _ hp (admissible_reply c mutant x.g _ _ _ _ (hp _) (by simp))
  · simp [JobBase,nojob]
  · simp [JobWritten,nojob]
  · simp [CheckedReal,nojob]

theorem admissible_draftEntry (g : Ghost P) (p : Fin P) (pg : Page)
    (hp : pg.held = true → Admissible g p pg.buf pg.base) :
    (draftEntry pg).bytes ≠ noneBytes →
    Admissible g p (draftEntry pg).bytes (draftEntry pg).base := by
  unfold draftEntry
  split
  · intro _; exact hp (by simp_all)
  · simp

theorem G_draftSync (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (p : Fin P) (hx : GInv x) : GInv (draftSync c x p) := by
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  unfold draftSync
  simp only [mutant,show ("none" == "MBD") = false from rfl,Bool.false_and,
    Bool.false_eq_true,ite_false]
  split
  · apply G_commit
    have hm := authMono_wrote x.g p [((draftEntry (x.s.pages p)).bytes,(draftEntry (x.s.pages p)).ver)]
    refine ⟨hb,?_,?_,?_,?_,hc⟩
    · intro q h; exact admissible_mono hm (hp q h)
    · intro q h
      by_cases he : q = p
      · subst q
        simp only [put,ite_true] at h ⊢
        exact admissible_mono hm (admissible_draftEntry x.g p _ (hp p) h)
      · simpa only [put,he,ite_false] using admissible_mono hm (hd q (by simpa [put,he] using h))
    · intro h; exact admissible_mono hm (hj h)
    · intro h ph; exact hm _ _ (hw h ph)
  · apply G_commit
    refine ⟨hb,hp,?_,hj,hw,hc⟩
    intro q h
    by_cases he : q = p
    · subst q
      simp only [put,ite_true] at h ⊢
      exact admissible_draftEntry x.g p _ (hp p) h
    · simpa [put,he] using hd q (by simpa [put,he] using h)

theorem G_switchReq (c : Config P) (x : State P) (hx : GInv x) : GInv (switchReq c x) := by
  apply G_commit
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  refine ⟨hb,?_,hd,hj,hw,hc⟩
  intro p
  dsimp only
  split <;> exact hp p

theorem G_down (c : Config P) (x : State P) (hx : GInv x) :
    GInv ⟨down c x.s,gDown x.g⟩ := by
  rcases hx with ⟨hb,_,hd,_,_,_⟩
  exact ⟨hb,by simp [PageBases,down],hd,by simp [JobBase,down,nojob],
    by simp [JobWritten,down,nojob],by simp [CheckedReal,down,nojob]⟩

theorem G_launch (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : GInv x) : GInv (launch c x) := by
  apply G_commit
  rcases hx with ⟨hb,_,hd,hj,hw,hc⟩
  refine ⟨hb,?_,hd,hj,hw,hc⟩
  intro p h
  change (restored c x p).held = true at h
  change Admissible _ _ (restored c x p).buf (restored c x p).base
  unfold restored at h ⊢
  simp only [mutant,show ("none" == "MRB") = false from rfl,Bool.false_and,
    Bool.false_eq_true,ite_false] at h ⊢
  split at h
  · simp at h
  · simp_all only [ite_false]
    exact hd p (by simpa using (by assumption : ¬(x.s.drafts p).bytes == noneBytes))

theorem G_execute_non_delivery (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (hx : GInv x) (hen : enabled c x a = true)
    (notDelivery : ∀ r, a ≠ .deliverUp r)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) : GInv (execute c x a) := by
  cases a <;> simp only [execute]
  all_goals simp only [enabled,Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq,bne_iff_ne] at hen
  case observe p => exact G_observe c mutant x p hx
  case flush p => exact G_flush c x p hx hen.1.1.1.2
  case check => exact G_check c mutant x hx hen.2 diskReal
  case rename => exact G_rename c x hx hen.1.2 hen.2
  case dirSync ok => exact G_dirSync c mutant x ok hx hen.1.2 hen.2
  case saveFail => exact G_saveFail c mutant x hx
  case draftSync p => exact G_draftSync c mutant x p hx
  case switchReq => exact G_switchReq c x hx
  case launch => exact G_launch c mutant x hx
  case deliverUp r => exact False.elim (notDelivery r rfl)
  case switchFin => exact G_commit c x _ _ _ (G_down c x hx)
  case crash => exact G_commit c x _ _ _ (G_down c x hx)
  case power keep =>
    apply G_commit
    rcases G_down c x hx with ⟨hb,hp,hd,hj,hw,hc⟩
    exact ⟨hb,hp,hd,hj,hw,hc⟩
  case wResolve p v =>
    apply G_commit
    exact G_frame x _ hx rfl rfl rfl rfl (authMono_seen x.g _ _) hx.1
  all_goals
    apply G_commit
    first
    | (exact hx)
    | (rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩; exact ⟨hb,hp,hd,hj,hw,hc⟩)

theorem G_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (h : GInv ⟨s,g⟩) : GInv ⟨ackMail s p ok v took,g⟩ := by
  cases ok <;> exact h

theorem G_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hx : GInv ⟨s,x.g⟩) : GInv (upOpen c x s m ok) := by
  simp only [upOpen]
  apply G_commit; apply G_ackMail
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  split <;> (try split)
  all_goals refine ⟨hb,?_,hd,hj,hw,hc⟩
  all_goals apply pageBases_put ⟨s,x.g⟩ m.p _ hp
  all_goals first | exact hp m.p | (intro _; exact Or.inr (Or.inl rfl))

theorem G_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (hx : GInv ⟨s,x.g⟩) : GInv (upClose c x s m) := by
  apply G_commit
  split
  · rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
    exact ⟨hb,pageBases_put ⟨s,x.g⟩ m.p _ hp (by simp),hd,hj,hw,hc⟩
  · exact hx

theorem G_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (hx : GInv ⟨s,x.g⟩) : GInv (upDiscard c x s m ok) := by
  simp only [upDiscard]
  split
  · apply G_commit
    rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
    exact ⟨by simpa [GTag] using hb,hp,hd,hj,hw,hc⟩
  · split
    · exact G_commit c x _ _ _ (G_ackMail _ _ _ _ _ _ hx)
    · apply G_commit; apply G_ackMail
      rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
      refine ⟨hb,?_,hd,hj,hw,hc⟩
      exact pageBases_put ⟨s,x.g⟩ m.p _ hp (fun _ => Or.inr (Or.inl rfl))

theorem admissible_submit (c : Config P) (mutant : c.mutant = "none")
    (g : Ghost P) (p : Fin P) (pg : Page) (t bv v : Int)
    (ha : pg.held = true → Admissible g p pg.buf pg.base)
    (auth : bv = pg.ver → Auth g p pg.buf) :
    (submitTo c pg t bv v).held = true →
    Admissible g p (submitTo c pg t bv v).buf (submitTo c pg t bv v).base := by
  simp only [submitTo,mutant]
  simp only [show ("none" == "MIS") = false from rfl,show ("none" == "MST") = false from rfl,
    Bool.false_eq_true,ite_false,Bool.or_false]
  split
  · intro hh
    rcases ha hh with h | h | h
    · exact Or.inl h
    · apply Or.inr; apply Or.inr; rw [h]; exact auth (by simpa using (by assumption : (bv == pg.ver) = true))
    · exact Or.inr (Or.inr h)
  · intro _; exact Or.inl rfl

theorem G_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (hx : GInv ⟨s,x.g⟩)
    (currentAuth : m.bv = (s.pages m.p).ver → Auth x.g m.p (s.pages m.p).buf)
    (resolveAuth : m.ro ≠ noneBytes → Auth x.g m.p m.ro) : GInv (upSubmit c x s m) := by
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  have hn : (if m.ro != noneBytes then
      {s.pages m.p with buf := m.t,base := m.ro,typed := true,ver := x.g.vc + 1, conflict := (s.pages m.p).obs != m.ro,risk := (s.pages m.p).risk || (s.pages m.p).obs != m.ro}
      else submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 1)).held = true →
      Admissible x.g m.p (if m.ro != noneBytes then m.t else (submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 1)).buf)
        (if m.ro != noneBytes then m.ro else (submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 1)).base) := by
    split
    · intro _; simp_all only [ite_true]
      exact Or.inr (Or.inr (resolveAuth (by simpa using (by assumption : (m.ro != noneBytes) = true))))
    · simp_all only [ite_false]
      exact admissible_submit c mutant x.g m.p _ _ _ _ (hp _) currentAuth
  simp only [upSubmit,mutant]
  simp only [show ("none" != "MIS") = true from rfl,show ("none" == "MDO") = false from rfl,
    Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · apply G_commit
    exact ⟨by simpa [GTag] using hb,hp,hd,hj,hw,hc⟩
  · apply G_commit; apply G_ackMail
    refine ⟨by simpa [GTag] using hb,?_,hd,hj,hw,hc⟩
    apply pageBases_put ⟨s,x.g⟩ m.p _ hp
    by_cases hr : (m.ro != noneBytes) = true
    · simpa only [hr,ite_true] using hn
    · simpa only [hr,Bool.false_eq_true,ite_false] using hn

theorem G_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (hx : GInv ⟨s,x.g⟩)
    (sourceAuth : m.bv = (s.pages m.p).ver → Auth x.g m.p (s.pages m.p).buf)
    (destAuth : m.bv2 = (s.pages m.q).ver → Auth x.g m.q (s.pages m.q).buf) :
    GInv (upOp c x s m ok) := by
  rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
  let na := submitTo c (s.pages m.p) m.t m.bv (x.g.vc + 2)
  let nb := {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc + 1) with risk := true}
  have ha : na.held = true → Admissible x.g m.p na.buf na.base :=
    admissible_submit c mutant x.g m.p _ _ _ _ (hp _) sourceAuth
  have hn : nb.held = true → Admissible x.g m.q nb.buf nb.base :=
    admissible_submit c mutant x.g m.q _ _ _ _ (hp _) destAuth
  have np : PageBases ⟨{s with pages := put (put s.pages m.q nb) m.p na},x.g⟩ :=
    pageBases_put ⟨{s with pages := put s.pages m.q nb},x.g⟩ m.p na
      (pageBases_put ⟨s,x.g⟩ m.q nb hp hn) ha
  have nd : DraftBases ⟨{s with drafts := put s.drafts m.q (draftEntry nb)},x.g⟩ := by
    intro p h
    by_cases he : p = m.q
    · subst p; simp only [put,ite_true] at h ⊢
      exact admissible_draftEntry x.g _ _ hn h
    · simpa [put,he] using hd p (by simpa [put,he] using h)
  have hm := authMono_wrote x.g m.q [(m.t2,x.g.vc + 1)]
  simp only [upOp,mutant]
  simp only [show ("none" == "MOD") = false from rfl,show ("none" != "MOD") = true from rfl,
    show ("none" != "MSM") = true from rfl,Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · apply G_commit
    exact ⟨by simpa [GTag] using hb,hp,hd,hj,hw,hc⟩
  · split
    · apply G_commit; apply G_ackMail; apply G_ackMail
      exact ⟨hb,hp,hd,hj,hw,hc⟩
    · apply G_commit; apply G_ackMail; apply G_ackMail
      refine ⟨by simpa [GTag] using hb,?_,?_,?_,?_,hc⟩
      · intro p h; exact admissible_mono hm (np p h)
      · intro p h; exact admissible_mono hm (nd p h)
      · intro h; exact admissible_mono hm (hj h)
      · intro h ph; exact hm _ _ (hw h ph)

/-- The outstanding protocol lemma is explicit here, separate from storage preservation. -/
def DeliveryAuth (x : State P) : Prop := ∀ m ∈ x.s.up,
  (m.kind ≠ "open" ∧ m.kind ≠ "close" ∧ m.kind ≠ "discard" ∧ m.kind ≠ "op" →
    (m.bv = (x.s.pages m.p).ver → Auth x.g m.p (x.s.pages m.p).buf) ∧
    (m.ro ≠ noneBytes → Auth x.g m.p m.ro)) ∧
  (m.kind = "op" →
    (m.bv = (x.s.pages m.p).ver → Auth x.g m.p (x.s.pages m.p).buf) ∧
    (m.bv2 = (x.s.pages m.q).ver → Auth x.g m.q (x.s.pages m.q).buf))

def authB (g : Ghost P) (p : Fin P) (b : Int) : Bool :=
  (g.seen p).contains b || (g.wrote p).any fun w => w.1 == b
def admissibleB (g : Ghost P) (p : Fin P) (b a : Int) : Bool :=
  a == unknown || a == b || authB g p a
def pageBasesB (x : State P) : Bool := (ids P).all fun p =>
  !(x.s.pages p).held || admissibleB x.g p (x.s.pages p).buf (x.s.pages p).base
def draftBasesB (x : State P) : Bool := (ids P).all fun p =>
  (x.s.drafts p).bytes == noneBytes || admissibleB x.g p (x.s.drafts p).bytes (x.s.drafts p).base
def jobBaseB (x : State P) : Bool :=
  !x.s.job.on || admissibleB x.g x.s.job.p x.s.job.bytes x.s.job.base
def jobWrittenB (x : State P) : Bool :=
  !x.s.job.on || x.s.job.phase != 3 || authB x.g x.s.job.p x.s.job.bytes
def checkedRealB (x : State P) : Bool :=
  !x.s.job.on || x.s.job.phase != 2 || !x.g.guard || x.s.job.base != unknown
def gInvB (x : State P) : Bool :=
  !x.g.bad.contains GTag && pageBasesB x && draftBasesB x && jobBaseB x && jobWrittenB x && checkedRealB x

@[simp] theorem authB_correct (g : Ghost P) (p : Fin P) (b : Int) :
    authB g p b = true ↔ Auth g p b := by
  simp [authB,Auth,List.any_eq_true]
@[simp] theorem admissibleB_correct (g : Ghost P) (p : Fin P) (b a : Int) :
    admissibleB g p b a = true ↔ Admissible g p b a := by
  simp [admissibleB,Admissible]; grind
@[simp] theorem pageBasesB_correct (x : State P) : pageBasesB x = true ↔ PageBases x := by
  simp [pageBasesB,PageBases,ids,List.all_eq_true]; grind
@[simp] theorem draftBasesB_correct (x : State P) : draftBasesB x = true ↔ DraftBases x := by
  simp [draftBasesB,DraftBases,ids,List.all_eq_true]; grind
@[simp] theorem jobBaseB_correct (x : State P) : jobBaseB x = true ↔ JobBase x := by
  simp [jobBaseB,JobBase]; grind
@[simp] theorem jobWrittenB_correct (x : State P) : jobWrittenB x = true ↔ JobWritten x := by
  simp [jobWrittenB,JobWritten]; grind
@[simp] theorem checkedRealB_correct (x : State P) : checkedRealB x = true ↔ CheckedReal x := by
  simp [checkedRealB,CheckedReal]; grind
@[simp] theorem gInvB_correct (x : State P) : gInvB x = true ↔ GInv x := by
  simp [gInvB,GInv]; grind

theorem G_deliverUp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (hx : GInv x) (hq : x.s.up = m::tail) (auth : DeliveryAuth x) :
    GInv (deliverUp c x m tail ok) := by
  have hm := auth m (by simp [hq])
  have hrest : GInv ⟨{x.s with up := tail},x.g⟩ := hx
  simp only [deliverUp]
  split
  · exact G_upOpen c x _ m ok hrest
  · split
    · exact G_upClose c x _ m hrest
    · split
      · exact G_upDiscard c x _ m ok hrest
      · split
        · have ha := hm.2 (by simpa using (by assumption : (m.kind == "op") = true))
          exact G_upOp c mutant x _ m ok hrest ha.1 ha.2
        · have hkind : m.kind ≠ "open" ∧ m.kind ≠ "close" ∧ m.kind ≠ "discard" ∧ m.kind ≠ "op" := by
            simp only [beq_iff_eq] at *
            exact ⟨by assumption,by assumption,by assumption,by assumption⟩
          have ha := hm.1 hkind
          exact G_upSubmit c mutant x _ m hrest ha.1 ha.2

theorem G_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (hx : GInv x) (hen : enabled c x a = true)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) (auth : DeliveryAuth x) :
    GInv (execute c x a) := by
  cases a with
  | deliverUp ok =>
    simp only [execute]
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail => exact G_deliverUp c mutant x m tail ok hx hq auth
  | _ => apply G_execute_non_delivery c mutant x _ hx hen (by intro r; simp) diskReal

theorem G_step (c : Config P) (mutant : c.mutant = "none")
    (x y : State P) (a : Action P) (hx : GInv x) (hs : step c x a = some y)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) (auth : DeliveryAuth x) : GInv y := by
  unfold step at hs
  split at hs
  · cases hs; exact G_execute c mutant x a hx (by assumption) diskReal auth
  · contradiction

/-- Conditional reduction, not an unconditional proof of clause G. -/
theorem G_reachable_of_deliveryAuth (c : Config P) (mutant : c.mutant = "none")
    (textsSafe : SafeTexts c)
    (protocol : ∀ x, Reachable c x → DeliveryAuth x)
    (x : State P) (hx : Reachable c x) : GInv x := by
  induction hx with
  | initial => exact G_init c
  | @advance x y hr a hs ih =>
    exact G_step c mutant x y a ih hs (disk_not_unknown_reachable c textsSafe x hr _) (protocol x hr)

theorem G_positiveTexts_of_deliveryAuth (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts T) (mutant : c.mutant = "none")
    (protocol : ∀ x, Reachable c x → DeliveryAuth x)
    (x : State P) (hx : Reachable c x) : "G-overwrote-unseen" ∉ x.g.bad :=
  (G_reachable_of_deliveryAuth c mutant (positiveTexts_safe c T texts) protocol x hx).1

#print axioms G_rename_safe
#print axioms G_rename
#print axioms G_execute_non_delivery
#print axioms G_upSubmit
#print axioms G_execute
#print axioms G_reachable_of_deliveryAuth
#print axioms G_positiveTexts_of_deliveryAuth
end Storage
