import Epochs
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def LivePromise (x : State P) (p : Fin P) : Prop :=
  (x.g.promise p).on = true ∧ ¬((x.g.promise p).saved = true ∧ (x.g.promise p).ep < x.g.ext p)
def Covers (x : State P) (p : Fin P) (b : Int) : Prop :=
  b = (x.g.promise p).bytes ∨ ∃ w ∈ x.g.wrote p, w.1 = b ∧ (x.g.promise p).ver < w.2

def NeedsDraft (c : Config P) (x : State P) (p : Fin P) : Prop :=
  LivePromise x p ∧ ((x.g.promise p).saved = false ∨ ¬Covers x p (x.s.disk p) ∨
    (c.weak = false ∧ ¬Covers x p (x.s.stable p)))
def Custody (c : Config P) (x : State P) : Prop := ∀ p, NeedsDraft c x p →
  (x.s.drafts p).bytes = (x.g.promise p).bytes ∧
  (x.s.alive = true → (x.s.pages p).held = true ∧ (x.s.pages p).risk = true)
def RecoverableCopies (x : State P) : Prop := ∀ p, LivePromise x p →
  Covers x p (x.s.disk p) ∨ Covers x p (x.s.drafts p).bytes

theorem covers_decide (x : State P) (p : Fin P) (b : Int) :
    (b == (x.g.promise p).bytes || (x.g.wrote p).any (fun w => w.1 == b && w.2 > (x.g.promise p).ver)) = true ↔
    Covers x p b := by simp [Covers]

theorem noLoss_iff_recoverable (x : State P) : noLoss x = true ↔
    x.s.alive = true ∨ RecoverableCopies x := by
  simp only [noLoss,Bool.or_eq_true,List.all_eq_true]
  apply or_congr Iff.rfl
  constructor
  · intro h p live
    have row := h p (by simp [ids])
    simp only [Bool.or_eq_true,Bool.and_eq_true,Bool.not_eq_true,decide_eq_true_eq] at row
    rcases row with (off | expired) | copy
    · exact False.elim (by have on := live.1; simp_all)
    · exact False.elim (live.2 expired)
    · obtain ⟨v,hv,hcov⟩ := List.any_eq_true.mp copy
      have cov := (covers_decide x p v).mp hcov
      have which : v = x.s.disk p ∨ v = (x.s.drafts p).bytes := by simpa [copies] using hv
      rcases which with rfl | rfl
      · exact Or.inl cov
      · exact Or.inr cov
  · intro h p _
    by_cases on : (x.g.promise p).on = true
    · by_cases expired : (x.g.promise p).saved = true ∧ (x.g.promise p).ep < x.g.ext p
      · simp [on,expired.1,expired.2]
      · right
        have live : LivePromise x p := ⟨on,expired⟩
        rcases h p live with disk | draft
        · exact List.any_eq_true.mpr ⟨x.s.disk p,by simp [copies],(covers_decide x p _).mpr disk⟩
        · exact List.any_eq_true.mpr ⟨(x.s.drafts p).bytes,by simp [copies],(covers_decide x p _).mpr draft⟩
    · simp [on]

theorem custody_recoverable (c : Config P) (x : State P) (h : Custody c x) : RecoverableCopies x := by
  intro p live
  by_cases disk : Covers x p (x.s.disk p)
  · exact Or.inl disk
  · exact Or.inr (Or.inl (h p ⟨live,Or.inr (Or.inl disk)⟩).1)

theorem noLoss_of_custody (c : Config P) (x : State P) (h : Custody c x) : noLoss x = true :=
  noLoss_iff_recoverable x |>.mpr (Or.inr (custody_recoverable c x h))

theorem noLoss_reachable_of_custody (c : Config P)
    (custody : ∀ x, Reachable c x → Custody c x) (x : State P) (hx : Reachable c x) : noLoss x = true :=
  noLoss_of_custody c x (custody x hx)

theorem custody_init (c : Config P) : Custody c (init c) := by simp [Custody,NeedsDraft,LivePromise,init]

theorem custody_frame (c : Config P) (x y : State P) (h : Custody c x)
    (promise : y.g.promise = x.g.promise) (ext : y.g.ext = x.g.ext) (wrote : y.g.wrote = x.g.wrote)
    (disk : y.s.disk = x.s.disk) (stable : y.s.stable = x.s.stable) (drafts : y.s.drafts = x.s.drafts)
    (risk : ∀ p, (x.s.alive = true → (x.s.pages p).held = true ∧ (x.s.pages p).risk = true) →
      y.s.alive = true → (y.s.pages p).held = true ∧ (y.s.pages p).risk = true) : Custody c y := by
  intro p need
  have old : NeedsDraft c x p := by simpa only [NeedsDraft,LivePromise,Covers,promise,ext,wrote,disk,stable] using need
  obtain ⟨eq,held⟩ := h p old
  exact ⟨by simpa only [drafts,promise] using eq,risk p held⟩

theorem custody_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Custody c ⟨s,g⟩) (same : ∀ p, PostPromises x s g p = g.promise p) :
    Custody c (commit c x s g n) := by
  apply custody_frame c ⟨s,g⟩ _ h
  · funext p; exact same p
  all_goals first | rfl | (intro p hp; exact hp)

theorem postPromises_same_pages (x : State P) (s : Sys P) (g : Ghost P)
    (same : s.pages = x.s.pages) : ∀ p, PostPromises x s g p = g.promise p := by
  intro p; simp [PostPromises,same,dirty]; grind

theorem postPromises_unheld_new (x : State P) (s : Sys P) (g : Ghost P)
    (unheld : ∀ p, (s.pages p).held = false) : ∀ p, PostPromises x s g p = g.promise p := by
  intro p; simp [PostPromises,unheld p]

theorem postPromises_unheld_old (x : State P) (s : Sys P) (g : Ghost P)
    (unheld : ∀ p, (x.s.pages p).held = false) : ∀ p, PostPromises x s g p = g.promise p := by
  intro p; simp [PostPromises,unheld p]

end Storage
