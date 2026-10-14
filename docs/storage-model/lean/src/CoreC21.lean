import Storage21
namespace Storage21
set_option maxRecDepth 4096
set_option maxHeartbeats 2000000

def CTag : String := "C-overwrote-external"
def Guarded (c : Config P) (x : State P) : Prop :=
  x.s.job.on = true → x.s.job.phase = 2 →
    x.g.guard = true ∧ (c.r1 = true ∨ x.s.disk x.s.job.p = x.s.job.base)
def CInv (c : Config P) (x : State P) : Prop := CTag ∉ x.g.bad ∧ Guarded c x

@[simp] theorem mem_union (a b : List String) (t : String) :
    t ∈ union a b ↔ t ∈ a ∨ t ∈ b := by
  induction b generalizing a with
  | nil => simp [union]
  | cons v vs ih =>
    simp only [union, List.foldl_cons]
    change t ∈ union (if a.contains v then a else a ++ [v]) vs ↔ _
    rw [ih]
    split <;> simp_all <;> grind

@[simp] theorem mem_tags (b : Bool) (t u : String) :
    t ∈ tags b u ↔ b = true ∧ t = u := by
  cases b <;> simp [tags]

@[simp] theorem C_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (name : String) :
    CInv c (commit c x s g name) ↔ CInv c ⟨s,g⟩ := by
  simp [CInv, Guarded, commit, CTag]

@[simp] theorem C_init (c : Config P) : CInv c (init c) := by
  simp [CInv, Guarded, init, nojob]

@[simp] theorem bad_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    CTag ∉ (commit c x s g n).g.bad ↔ CTag ∉ g.bad := by simp [commit,CTag]
@[simp] theorem bad_literal_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    "C-overwrote-external" ∉ (commit c x s g n).g.bad ↔ "C-overwrote-external" ∉ g.bad := by
  exact bad_commit c x s g n
@[simp] theorem job_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).s.job = s.job := rfl
@[simp] theorem disk_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).s.disk = s.disk := rfl
@[simp] theorem guard_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).g.guard = g.guard := rfl

@[simp] theorem job_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool) :
    (ackMail s p ok v took).job = s.job := by cases ok <;> rfl
@[simp] theorem disk_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool) :
    (ackMail s p ok v took).disk = s.disk := by cases ok <;> rfl

theorem C_frame (c : Config P) (x y : State P)
    (hj : y.s.job = x.s.job) (hd : y.s.disk = x.s.disk)
    (hg : y.g.guard = x.g.guard) (hb : CTag ∉ y.g.bad ↔ CTag ∉ x.g.bad)
    (hx : CInv c x) : CInv c y := by
  simpa only [CInv,Guarded,hj,hd,hg,hb] using hx

-- Each request preserves C's concrete fields. Only non-C transition tags are added.
theorem C_upOpen (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool)
    (hj : sys.job = x.s.job) (hd : sys.disk = x.s.disk) (hx : CInv c x) :
    CInv c (upOpen c x sys m rok) := by
  apply C_frame c x _ _ _ _ _ hx
  · simp only [upOpen,job_commit,job_ackMail]; exact hj
  · simp only [upOpen,disk_commit,disk_ackMail]; exact hd
  · simp only [upOpen,guard_commit]; split <;> split <;> rfl
  · simp only [upOpen,bad_commit]; split <;> split <;> rfl

theorem C_upClose (c : Config P) (x : State P) (sys : Sys P) (m : Up P)
    (hj : sys.job = x.s.job) (hd : sys.disk = x.s.disk) (hx : CInv c x) :
    CInv c (upClose c x sys m) := by
  apply C_frame c x _ _ _ _ _ hx
  · simp only [upClose,job_commit]; split <;> exact hj
  · simp only [upClose,disk_commit]; split <;> exact hd
  · simp only [upClose,guard_commit]
  · simp only [upClose,bad_commit]

theorem C_upDiscard (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool)
    (hj : sys.job = x.s.job) (hd : sys.disk = x.s.disk) (hx : CInv c x) :
    CInv c (upDiscard c x sys m rok) := by
  simp only [upDiscard]
  split
  · simp only [C_commit]
    exact ⟨by simpa [CTag] using hx.1, by simpa [Guarded,hj,hd] using hx.2⟩
  · split <;> simp only [C_commit]
    all_goals constructor
    all_goals first
      | exact hx.1
      | simpa only [Guarded,job_ackMail,disk_ackMail,hj,hd] using hx.2

theorem C_upSubmit (c : Config P) (x : State P) (sys : Sys P) (m : Up P)
    (hj : sys.job = x.s.job) (hd : sys.disk = x.s.disk) (hx : CInv c x) :
    CInv c (upSubmit c x sys m) := by
  simp only [upSubmit]
  split <;> simp only [C_commit]
  all_goals constructor
  all_goals first
    | simpa [CTag] using hx.1
    | simpa only [Guarded,job_ackMail,disk_ackMail,hj,hd] using hx.2

theorem C_upOp (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool)
    (hj : sys.job = x.s.job) (hd : sys.disk = x.s.disk) (hx : CInv c x) :
    CInv c (upOp c x sys m rok) := by
  simp only [upOp]
  split
  · simp only [C_commit]
    exact ⟨by simpa [CTag] using hx.1, by simpa [Guarded,hj,hd] using hx.2⟩
  · split <;> simp only [C_commit]
    all_goals constructor
    all_goals first
      | simpa [CTag] using hx.1
      | simpa only [Guarded,job_ackMail,disk_ackMail,hj,hd] using hx.2

-- Preservation is checked on the concrete action code, including all non-C ghosts.
theorem C_execute (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (hx : CInv c x) (hen : enabled c x a = true) :
    CInv c (execute c x a) := by
  rcases hx with ⟨hbad,hguard⟩
  cases a <;> simp only [execute]
  all_goals
    simp only [enabled, Bool.and_eq_true, Bool.or_eq_true,
      beq_iff_eq, bne_iff_ne] at hen
  all_goals
    first
    | (simp only [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,
        observe,switchReq,windowCrash,launch,C_commit]
       exact ⟨hbad,hguard⟩)
    | skip

  case flush p =>
    simp only [flush,C_commit]
    simp [CInv,Guarded,hbad]
  case dirSync ok =>
    simp only [dirSync,C_commit]
    simp [CInv,Guarded,nojob,hbad]
  case saveFail =>
    simp only [saveFail,C_commit]
    simp [CInv,Guarded,nojob,hbad]
  case switchFin =>
    simp only [switchFin,C_commit]
    simp [CInv,Guarded,down,gDown,nojob,hbad]
  case crash =>
    simp only [crash,C_commit]
    simp [CInv,Guarded,down,gDown,nojob,hbad]
  case power keep =>
    simp only [power,C_commit]
    simp [CInv,Guarded,down,gDown,nojob,hbad]
  case draftSync p =>
    simp [draftSync,CInv,Guarded,mutant]
    split <;> exact ⟨hbad,hguard⟩
  case check =>
    simp only [check,mutant]
    split
    · simp only [C_commit]
      simp [CInv,Guarded,nojob,hbad]
    · simp only [C_commit]
      simp_all [CInv,Guarded]
  case rename =>
    have hg := hguard hen.1.2 hen.2
    simp only [CTag] at hbad
    simp [rename,commit,CInv,Guarded,CTag,hg.1,hbad]
    grind
  case extWriteD p v dur =>
    simp only [extWriteD,C_commit]
    constructor
    · exact hbad
    · intro hon hphase
      change x.s.job.on = true at hon
      change x.s.job.phase = 2 at hphase
      have hg := hguard hon hphase
      rcases hg.2 with hr | hd
      · exact ⟨hg.1,Or.inl hr⟩
      · refine ⟨hg.1,?_⟩
        by_cases heq : x.s.job.p = p
        · have hr : c.r1 = true := by
            have hw : window x.s p = true := by simp [window,hon,hphase,← heq]
            rcases hen.1.2 with hh | hr
            · simp [hw] at hh
            · exact hr
          exact Or.inl hr
        · exact Or.inr (by simpa [put,heq] using hd)
  case deliverUp rok =>
    cases hu : x.s.up with
    | nil => exact ⟨hbad,hguard⟩
    | cons m tail =>
      simp only [deliverUp]
      split <;> try split <;> try split <;> try split
      all_goals first
        | exact C_upOpen c x _ m rok rfl rfl ⟨hbad,hguard⟩
        | exact C_upClose c x _ m rfl rfl ⟨hbad,hguard⟩
        | exact C_upDiscard c x _ m rok rfl rfl ⟨hbad,hguard⟩
        | exact C_upOp c x _ m rok rfl rfl ⟨hbad,hguard⟩
        | exact C_upSubmit c x _ m rfl rfl ⟨hbad,hguard⟩

inductive Reachable (c : Config P) : State P → Prop where
  | initial : Reachable c (init c)
  | advance {x y : State P} (hx : Reachable c x) (a : Action P)
      (hstep : step c x a = some y) : Reachable c y

theorem C_step (c : Config P) (x y : State P) (a : Action P)
    (mutant : c.mutant = "none") (hx : CInv c x) (hs : step c x a = some y) :
    CInv c y := by
  unfold step at hs
  split at hs
  · cases hs
    exact C_execute c x a mutant hx (by assumption)
  · contradiction

theorem C_reachable_invariant (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : CInv c x := by
  induction hx with
  | initial => exact C_init c
  | advance hr a hs ih => exact C_step c _ _ a mutant ih hs

/-- Exact absence of Quint's C tag, with no bound on pages, texts, or histories. -/
theorem C_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : "C-overwrote-external" ∉ x.g.bad :=
  (C_reachable_invariant c mutant x hx).1

/-- The stronger phase-2 statement explains both the guarantee and its R1 exception. -/
theorem C_guard_at_rename (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) (hon : x.s.job.on = true) (hp : x.s.job.phase = 2) :
    x.g.guard = true ∧ (c.r1 = true ∨ x.s.disk x.s.job.p = x.s.job.base) :=
  (C_reachable_invariant c mutant x hx).2 hon hp

#print axioms C_reachable
#print axioms C_guard_at_rename
#print axioms C_execute
end Storage21

