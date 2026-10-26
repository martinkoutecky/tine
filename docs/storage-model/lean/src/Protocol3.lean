import Client3
import CoreLifetimeClosure21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def Kinds (s : Storage21.Sys P) : Prop := ∀ m ∈ s.up, m.kind ∈ ["open","close","discard","submit","op"]

theorem kinds_append (s : Storage21.Sys P) (m : Up P) (h : Kinds s)
    (kind : m.kind ∈ ["open","close","discard","submit","op"]) :
    ∀ u ∈ s.up ++ [m], u.kind ∈ ["open","close","discard","submit","op"] := by
  intro u hu
  rcases List.mem_append.mp hu with hu | hu
  · exact h u hu
  · simp only [List.mem_singleton] at hu; subst u; exact kind

theorem kinds_execute21 (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : Kinds x.s) : Kinds (Storage21.execute c x a).s := by
  cases a <;> simp only [Storage21.execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have ht : ∀ u ∈ tail, u.kind ∈ ["open","close","discard","submit","op"] := by
        intro u hu; exact h u (by simp [hq,hu])
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upSubmit,Storage21.upOp]
      all_goals (try split) <;> (try split)
      all_goals simpa [Kinds,Storage21.commit,up_ackMail] using ht
  case wOpen p =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case wClose p =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case wDiscard p =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case wSend p =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case wResolve p t =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case wOp p q t u =>
    apply kinds_append _ _ h
    simp [Storage21.up1]
  case windowCrash =>
    simp only [Storage21.windowCrash]; split
    · simp [Kinds,Storage21.commit]
    · intro u hu
      obtain ⟨m,hm,rfl⟩ := List.mem_map.mp hu
      exact h m hm
  case check => simp only [Storage21.check]; split <;> exact h
  case draftSync p => simp only [Storage21.draftSync]; split <;> (try split) <;> exact h
  case switchFin => simp [Kinds,Storage21.switchFin,Storage21.commit,Storage21.down]
  case crash => simp [Kinds,Storage21.crash,Storage21.commit,Storage21.down]
  case power keep => simp [Kinds,Storage21.power,Storage21.commit,Storage21.down]
  all_goals exact h

theorem kinds_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : Kinds x.s.toSys) : Kinds (execute c x a).s.toSys := by
  cases a with
  | old a =>
    have core := kinds_execute21 c (project x) a h
    cases a <;> simp only [execute,executeOld]
    case rename => exact h
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact h
  | opRename src dst refs rt => exact h
  | flushDel p => exact h

theorem inactiveClient_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : InactiveClient (project x))
    (hen : enabled c x a = true) : InactiveClient (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := inactiveClient_execute c (project x) a h hen
    cases a <;> simp only [execute,executeOld]
    case rename =>
      have live : x.s.alive = true := by simp [enabled,Storage21.enabled,project] at hen; grind
      simp [rename,commit,project,Storage21.commit,InactiveClient,live]
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [execute,opDelete,commit,project,Storage21.commit,InactiveClient,live]
  | opRename src dst refs rt =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [execute,opRename,commit,project,Storage21.commit,InactiveClient,live]
  | flushDel p =>
    have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp [execute,flushDel,lift,project,Storage21.flush,Storage21.commit,InactiveClient,live]

theorem lifetime_commit_grow (c : Config P) (x : Storage21.State P) (s : Storage21.Sys P)
    (g : Storage21.Ghost P) (n : String) (h : Lifetime x.s)
    (live : s.alive = true) (w : s.w = x.s.w) (mb : s.mb = x.s.mb) (up : s.up = x.s.up)
    (grow : ∀ p, (x.s.pages p).held = true → (s.pages p).held = true) :
    Lifetime (Storage21.commit c x s g n).s := by
  constructor
  · simpa [Storage21.commit,up] using h.ordered
  · simpa [Storage21.commit,up] using h.last
  · intro m hm p need; exact grow p (h.held m (by simpa [Storage21.commit,up] using hm) p need)
  · intro p hon
    obtain ⟨held,active⟩ := h.active p (by simpa [Storage21.commit,w] using hon)
    exact ⟨grow p held,by simpa [Storage21.commit,up] using active⟩
  · simpa [Storage21.commit,w] using h.pending
  · intro m hm p cur
    obtain ⟨sent,neg⟩ := h.waiting m (by simpa [Storage21.commit,up] using hm) p cur
    constructor
    · simpa [Storage21.commit,w] using sent
    · change (if _ then _ else s.mb p).ack < 0
      split
      · split <;> (first | simpa [mb] using neg | (dsimp only; omega))
      · simpa [mb] using neg
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    have old : (x.s.mb p).ack ≥ 0 := by
      split at pos
      · split at pos
        · simpa [mb] using pos
        · dsimp only at pos; omega
      · simpa [mb] using pos
    simpa [Storage21.commit,w,up] using h.done p old
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    change (if _ then _ else s.mb p).on = true
    split <;> simp_all only
    simpa [mb] using h.mailOn p (by simpa [mb] using pos)
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    have old : (x.s.mb p).ack ≥ 0 := by
      split at pos
      · split at pos
        · simpa [mb] using pos
        · dsimp only at pos; omega
      · simpa [mb] using pos
    change (if _ then _ else s.mb p).held = (s.pages p).held
    split
    · rfl
    · have sent := (h.done p old).1
      have same : (x.s.pages p).held = (s.pages p).held := by
        have no : ¬ (s.alive && ((s.w p).on || (s.w p).sent) &&
          ((x.s.pages p).ver != (s.pages p).ver || (x.s.pages p).buf != (s.pages p).buf ||
            (x.s.pages p).obs != (s.pages p).obs || (x.s.pages p).conflict != (s.pages p).conflict ||
            (x.s.pages p).held != (s.pages p).held)) = true := by assumption
        simp [live,w,sent] at no
        grind
      simpa [mb,same] using h.mailHeld p old

theorem opEdit_held (s : Sys P) (p : Fin P) (t v : Int) : (opEdit s p t v).held = true := by
  simp only [opEdit]; split
  · simpa using (by assumption : (s.pages p).held = true)
  · rfl

theorem grow_put (old pages : Fin P → Page) (p : Fin P) (pg : Page)
    (h : ∀ q, (old q).held = true → (pages q).held = true) (held : pg.held = true) :
    ∀ q, (old q).held = true → (put pages p pg q).held = true := by
  intro q hq; by_cases eq : q = p <;> simp_all [put]

theorem grow_fold (old pages : Fin P → Page) (refs : List (Fin P)) (nr : Fin P → Page)
    (h : ∀ p, (old p).held = true → (pages p).held = true) (held : ∀ p, (nr p).held = true) :
    ∀ p, (old p).held = true → (refs.foldl (fun acc r => put acc r (nr r)) pages p).held = true := by
  induction refs generalizing pages with
  | nil => exact h
  | cons r tail ih => exact ih _ (grow_put old pages r (nr r) h (held r))

theorem lifetime_new (c : Config P) (x : State P) (a : Action P)
    (h : Lifetime x.s.toSys) (hen : enabled c x a = true)
    (isnew : (∃ src dst refs rt, a = .opRename src dst refs rt) ∨ (∃ p, a = .opDelete p)) :
    Lifetime (execute c x a).s.toSys := by
  rcases isnew with ⟨src,dst,refs,rt,rfl⟩ | ⟨p,rfl⟩
  · have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp only [execute,opRename,commit]
    apply lifetime_commit_grow c (project x) _ _ _ h
    · exact live
    · rfl
    · rfl
    · rfl
    · apply grow_fold
      · split
        · apply grow_put
          · apply grow_put
            · intro p hp; exact hp
            · simp only [opEdit_held]
          · simp only [opEdit_held]
        · intro p hp; exact hp
      · intro p; simp only [opEdit_held]
  · have live : x.s.alive = true := by simp [enabled] at hen; grind
    simp only [execute,opDelete,commit]
    apply lifetime_commit_grow c (project x) _ _ _ h
    · exact live
    · rfl
    · rfl
    · rfl
    · apply grow_put
      · intro p hp; exact hp
      · simp only [opEdit_held]

structure ProtocolInv3 (x : State P) : Prop where
  kinds : Kinds x.s.toSys
  inactive : InactiveClient (project x)
  life : Lifetime x.s.toSys
  carriage : Carriage (project x)

theorem lifetime_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ProtocolInv3 x) (hen : enabled c x a = true) :
    Lifetime (execute c x a).s.toSys := by
  cases a with
  | old a =>
    have core := lifetime_execute c mutant (project x) a h.life hen h.kinds
      (by intro eq; subst a; exact lifetime_launch c (project x) h.inactive hen)
    cases a <;> simp only [execute,executeOld]
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact lifetime_new c x (.opDelete p) h.life hen (Or.inr ⟨p,rfl⟩)
  | opRename src dst refs rt => exact lifetime_new c x (.opRename src dst refs rt) h.life hen (Or.inl ⟨src,dst,refs,rt,rfl⟩)
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    apply lifetime_commit
    exact lifetime_frame x.s.toSys _ h.life (fun _ => rfl) rfl rfl rfl

theorem carriage_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ProtocolInv3 x) : Carriage (project (execute c x a)) := by
  cases a with
  | old a =>
    have core : Carriage (Storage21.execute c (project x) a) := by
      by_cases delivery : ∃ ok, a = .deliverUp ok
      · obtain ⟨ok,rfl⟩ := delivery
        cases hq : x.s.up with
        | nil => simpa [Storage21.execute,project,hq] using h.carriage
        | cons m tail =>
          simpa [Storage21.execute,project,hq] using carriage_deliverUp c (project x) m tail ok h.carriage hq
            (lifetime_headHeld (project x) h.life) (h.kinds m (by simp [hq]))
      · exact carriage_execute_non_delivery c mutant (project x) a h.carriage (by simpa using delivery)
    cases a <;> simp only [execute,executeOld]
    case rename => exact h.carriage
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact h.carriage
  | opRename src dst refs rt => exact h.carriage
  | flushDel p => exact h.carriage

theorem protocol_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ProtocolInv3 x) (hen : enabled c x a = true) :
    ProtocolInv3 (execute c x a) :=
  ⟨kinds_execute3 c x a mutant h.kinds, inactiveClient_execute3 c x a mutant h.inactive hen,
    lifetime_execute3 c x a mutant h hen,carriage_execute3 c x a mutant h⟩

theorem protocol_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : ProtocolInv3 x := by
  induction hr with
  | initial => constructor <;> simp [Kinds,InactiveClient,Carriage,QueueCarries,initWith,embed,project,Storage21.init]
               constructor <;> simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact protocol_execute3 c x a mutant ih (by assumption)
    · contradiction

theorem accepted_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : accepted x = true :=
  accepted_of_carriage (project x) (protocol_reachable3 c disk x mutant hr).carriage

theorem mailFence_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : MailFence (project x) := by
  have hp := protocol_reachable3 c disk x mutant hr
  exact lifetime_mailFence (project x) hp.life hp.carriage

theorem Bprime1_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : "B'-window-replaced" ∉ x.g.bad :=
  Bprime1_reachable3_of_fence c disk mutant (fun x hx => mailFence_reachable3 c disk x mutant hx) x hr
end Storage3

