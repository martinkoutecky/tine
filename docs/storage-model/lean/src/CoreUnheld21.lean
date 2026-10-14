import CoreLifetimeClosure21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
@[simp] theorem unheld_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    "request-to-unheld" ∉ (commit c x s g n).g.bad ↔ "request-to-unheld" ∉ g.bad := by
  simp [commit]

theorem unheld_execute (c : Config P) (x : State P) (a : Action P)
    (h : "request-to-unheld" ∉ x.g.bad) (life : Lifetime x.s) (kinds : ∀ m ∈ x.s.up, m.kind ∈ ["open","close","discard","submit","op"]) :
    "request-to-unheld" ∉ (execute c x a).g.bad := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have mem : m ∈ x.s.up := by simp [hq]
      have kinds : m.kind = "open" ∨ m.kind = "close" ∨ m.kind = "discard" ∨ m.kind = "submit" ∨ m.kind = "op" := by
        simpa using kinds m mem
      rcases kinds with kind | kind | kind | kind | kind
      · simp only [deliverUp,kind]; simp
        simp only [upOpen,unheld_commit]; split <;> (try split) <;> exact h
      · simp only [deliverUp,kind]; simp
        simp only [upClose,unheld_commit]; exact h
      · have held := life.held m mem m.p (by simp [Needs,kind])
        simp [deliverUp,kind,upDiscard,held]
        split <;> simpa using h
      · have held := life.held m mem m.p (by simp [Needs,kind])
        simpa [deliverUp,kind,upSubmit,held] using h
      · have src := life.held m mem m.p (by simp [Needs,kind])
        have dst := life.held m mem m.q (by simp [Needs,kind])
        simp [deliverUp,kind,upOp,src,dst]
        split <;> (try split) <;> simpa using h
  case check => simp only [check]; split <;> simpa using h
  case rename => simpa [rename,commit] using h
  case draftSync p => simp only [draftSync,unheld_commit]; split <;> (try split) <;> exact h
  all_goals simpa [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,observe,flush,dirSync,
    saveFail,draftSync,switchReq,switchFin,extWriteD,crash,windowCrash,power,launch,commit,gDown] using h


end Storage21
