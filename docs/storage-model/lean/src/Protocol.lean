import ProofG
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

theorem authMono_trans (g h k : Ghost P) (ha : AuthMono g h) (hb : AuthMono h k) :
    AuthMono g k := fun p b h => hb p b (ha p b h)

theorem authMono_committed (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : AuthMono x.g g) : AuthMono x.g (commit c x s g n).g :=
  authMono_trans _ _ _ h (Auth_mono_commit c x s g n)

theorem authMono_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upOpen c x s m ok).g := by
  apply authMono_committed
  split <;> (try split)
  all_goals exact authMono_refl x.g
theorem authMono_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    AuthMono x.g (upClose c x s m).g := by
  exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upDiscard c x s m ok).g := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    AuthMono x.g (upSubmit c x s m).g := by
  simp only [upSubmit]
  split
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)
theorem authMono_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    AuthMono x.g (upOp c x s m ok).g := by
  simp only [upOp]
  split
  · exact authMono_committed c x _ _ _ (authMono_refl x.g)
  · split
    · exact authMono_committed c x _ _ _ (authMono_refl x.g)
    · exact authMono_committed c x _ _ _ (authMono_wrote x.g _ _)

/-- No crash, restore, power, queue, or mutant branch erases byte authorization. -/
theorem authMono_execute (c : Config P) (x : State P) (a : Action P) :
    AuthMono x.g (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact authMono_refl x.g
    | cons m tail =>
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact authMono_upOpen c x _ m ok
        | exact authMono_upClose c x _ m
        | exact authMono_upDiscard c x _ m ok
        | exact authMono_upOp c x _ m ok
        | exact authMono_upSubmit c x _ m
  case wResolve p v => exact authMono_committed c x _ _ _ (authMono_seen x.g _ _)
  case rename => exact authMono_committed c x _ _ _ (authMono_wrote x.g _ _)
  case draftSync p =>
    apply authMono_committed
    split <;> (try split)
    all_goals first | exact authMono_wrote x.g _ _ | exact authMono_refl x.g
  case check =>
    simp only [check]
    split <;> exact authMono_committed c x _ _ _ (authMono_refl x.g)
  all_goals exact authMono_committed c x _ _ _ (authMono_refl x.g)

theorem authMono_step (c : Config P) (x y : State P) (a : Action P)
    (hs : step c x a = some y) : AuthMono x.g y.g := by
  unfold step at hs
  split at hs
  · cases hs; exact authMono_execute c x a
  · contradiction

inductive Steps (c : Config P) : State P → State P → Prop where
  | refl (x) : Steps c x x
  | advance {x y z} (h : Steps c x y) (a : Action P) (hs : step c y a = some z) : Steps c x z

theorem authMono_steps (c : Config P) (x y : State P) (hs : Steps c x y) : AuthMono x.g y.g := by
  induction hs with
  | refl => exact authMono_refl x.g
  | advance h a hs ih => exact authMono_trans _ _ _ ih (authMono_step c _ _ a hs)

theorem vc_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).g.vc = g.vc := rfl

theorem vc_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    x.g.vc ≤ (upOpen c x s m ok).g.vc := by
  simp only [upOpen,vc_commit]
  split <;> (try split) <;> simp_all <;> omega
theorem vc_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    x.g.vc ≤ (upClose c x s m).g.vc := by simp only [upClose,vc_commit]; omega
theorem vc_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    x.g.vc ≤ (upDiscard c x s m ok).g.vc := by
  simp only [upDiscard]; split <;> (try split)
  all_goals simp only [vc_commit]; omega
theorem vc_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    x.g.vc ≤ (upSubmit c x s m).g.vc := by
  simp only [upSubmit]; split
  all_goals simp only [vc_commit]; omega
theorem vc_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    x.g.vc ≤ (upOp c x s m ok).g.vc := by
  simp only [upOp]; split <;> (try split)
  all_goals simp only [vc_commit]; omega

theorem vc_execute (c : Config P) (x : State P) (a : Action P) :
    x.g.vc ≤ (execute c x a).g.vc := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => simp
    | cons m tail =>
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact vc_upOpen c x _ m ok
        | exact vc_upClose c x _ m
        | exact vc_upDiscard c x _ m ok
        | exact vc_upOp c x _ m ok
        | exact vc_upSubmit c x _ m
  case observe p => simp only [observe,vc_commit]; split <;> omega
  case check => simp only [check]; split
                all_goals simp only [vc_commit]; (try split) <;> omega
  case launch => simp only [launch,vc_commit]; omega
  case draftSync p =>
    simp only [draftSync,vc_commit]
    split <;> (try split) <;> (try dsimp only) <;> omega
  all_goals simp only [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,flush,rename,
    dirSync,saveFail,draftSync,switchReq,switchFin,extWriteD,crash,windowCrash,power,vc_commit,down,gDown]
  all_goals (try split) <;> omega

theorem vc_step (c : Config P) (x y : State P) (a : Action P) (hs : step c x a = some y) :
    x.g.vc ≤ y.g.vc := by
  unfold step at hs
  split at hs
  · cases hs; exact vc_execute c x a
  · contradiction

theorem vc_nonnegative_reachable (c : Config P) (x : State P) (hx : Reachable c x) : 0 ≤ x.g.vc := by
  induction hx with
  | initial => simp [init]
  | advance h a hs ih => exact Int.le_trans ih (vc_step c _ _ a hs)

theorem job_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    (upOpen c x s m ok).s.job = s.job := by simp only [upOpen,job_commit,job_ackMail]
theorem job_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    (upClose c x s m).s.job = s.job := by simp only [upClose,job_commit]; split <;> rfl
theorem job_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    (upDiscard c x s m ok).s.job = s.job := by
  simp only [upDiscard]; split <;> (try split)
  all_goals simp only [job_commit,job_ackMail]
theorem job_upSubmit (c : Config P) (x : State P) (s : Sys P) (m : Up P) :
    (upSubmit c x s m).s.job = s.job := by
  simp only [upSubmit]; split
  all_goals simp only [job_commit,job_ackMail]
theorem job_upOp (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool) :
    (upOp c x s m ok).s.job = s.job := by
  simp only [upOp]; split <;> (try split)
  all_goals simp only [job_commit,job_ackMail]
theorem job_deliverUp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) :
    (deliverUp c x m tail ok).s.job = x.s.job := by
  simp only [deliverUp]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals first
    | exact job_upOpen c x _ m ok
    | exact job_upClose c x _ m
    | exact job_upDiscard c x _ m ok
    | exact job_upOp c x _ m ok
    | exact job_upSubmit c x _ m

theorem written_execute (c : Config P) (x : State P) (a : Action P) (hx : JobWritten x) :
    JobWritten (execute c x a) := by
  have ha := authMono_execute c x a
  cases a <;> simp only [execute]
  case rename =>
    intro _ _
    exact Auth_mono_commit c x _ _ _ _ _ (auth_wrote_added x.g _ _ _)
  case flush p => simp [flush,JobWritten,job_commit]
  case check =>
    simp only [check]; split
    · simp [JobWritten,job_commit,nojob]
    · simp [JobWritten,job_commit]
  case dirSync ok => simp [dirSync,JobWritten,job_commit,nojob]
  case saveFail => simp [saveFail,JobWritten,job_commit,nojob]
  case switchFin => simp [switchFin,JobWritten,job_commit,down,nojob]
  case crash => simp [crash,JobWritten,job_commit,down,nojob]
  case power keep => simp [power,JobWritten,job_commit,down,nojob]
  case deliverUp ok =>
    simp only [execute] at ha
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail =>
      intro h ph
      have hj := job_deliverUp c x m tail ok
      rw [hj] at h ph ⊢
      simp only [hq] at ha
      exact ha _ _ (hx h ph)
  all_goals
    intro h ph
    exact ha _ _ (hx h ph)

theorem written_reachable (c : Config P) (x : State P) (hx : Reachable c x) : JobWritten x := by
  induction hx with
  | initial => simp [JobWritten,init,nojob]
  | advance hr a hs ih =>
    unfold step at hs
    split at hs
    · cases hs; exact written_execute c _ a ih
    · contradiction

theorem checked_execute (c : Config P) (x : State P) (a : Action P) (hx : CheckedReal x)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) : CheckedReal (execute c x a) := by
  cases a <;> simp only [execute]
  case rename => simp [rename,CheckedReal,job_commit]
  case flush p => simp [flush,CheckedReal,job_commit]
  case check =>
    simp only [check]; split
    · simp [CheckedReal,job_commit,nojob]
    · intro _ _ hg
      change (x.s.disk x.s.job.p == x.s.job.base) = true at hg
      have he : x.s.disk x.s.job.p = x.s.job.base := by simpa using hg
      simpa [← he] using diskReal
  case dirSync ok => simp [dirSync,CheckedReal,job_commit,nojob]
  case saveFail => simp [saveFail,CheckedReal,job_commit,nojob]
  case switchFin => simp [switchFin,CheckedReal,job_commit,down,nojob]
  case crash => simp [crash,CheckedReal,job_commit,down,nojob]
  case power keep => simp [power,CheckedReal,job_commit,down,nojob]
  case draftSync p =>
    simp only [draftSync]; split <;> (try split)
    all_goals exact hx
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail =>
      have hj := job_deliverUp c x m tail ok
      simp only [deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split)
      all_goals simp only [CheckedReal,job_commit,job_ackMail,guard_commit]
      all_goals (try split) <;> (try split)
      all_goals exact hx
  all_goals exact hx

theorem checked_reachable (c : Config P) (texts : SafeTexts c) (x : State P)
    (hx : Reachable c x) : CheckedReal x := by
  induction hx with
  | initial => simp [CheckedReal,init,nojob]
  | @advance x y hr a hs ih =>
    have hd := disk_not_unknown_reachable c texts x hr x.s.job.p
    unfold step at hs
    split at hs
    · cases hs; exact checked_execute c x a ih hd
    · contradiction

#print axioms authMono_steps
#print axioms vc_nonnegative_reachable
#print axioms written_reachable
#print axioms checked_reachable
end Storage
