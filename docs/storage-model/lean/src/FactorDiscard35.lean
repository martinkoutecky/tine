import SilentRestore35
namespace Storage21
set_option maxHeartbeats 8000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false

theorem union_repeat10 [BEq α] [LawfulBEq α] (a b : List α) :
    union (union a b) b = union a b := by
  apply idle_union9
  intro q hq
  exact (mem_union_general _ _ _).mpr (Or.inr hq)

theorem commit_promise_named10 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P)
    (ps : List (Fin P)) (name : String)
    (unchanged : ∀ p,p ∉ ps → s.pages p = x.s.pages p)
    (named : ∀ p ∈ ps,p ∈ g.mine) :
    (commit c x s g name).g.promise = g.promise := by
  funext p
  by_cases mem : p ∈ ps
  · simp [commit,List.contains_iff_mem,named p mem]
  · simp only [commit,unchanged p mem]
    cases hc : clean (x.s.pages p) <;> simp [dirty,hc]

theorem sys_eq10 (x y : Sys P)
    (alive : x.alive = y.alive) (disk : x.disk = y.disk) (stable : x.stable = y.stable)
    (drafts : x.drafts = y.drafts) (pages : x.pages = y.pages) (job : x.job = y.job)
    (w : x.w = y.w) (mb : x.mb = y.mb) (up : x.up = y.up) : x = y := by
  cases x; cases y; simp_all
theorem ghost_eq10 (x y : Ghost P)
    (vc : x.vc = y.vc) (promise : x.promise = y.promise) (epochs : x.ext = y.ext)
    (wrote : x.wrote = y.wrote) (owed : x.owed = y.owed) (guard : x.guard = y.guard)
    (seen : x.seen = y.seen) (mine : x.mine = y.mine) (bad : x.bad = y.bad) : x = y := by
  cases x; cases y; simp_all
end Storage21
namespace Storage3
open Storage21
theorem state_eq10 (x y : State P) (sys : x.s.toSys = y.s.toSys)
    (trash : x.s.trash = y.s.trash) (trashStable : x.s.trashStable = y.s.trashStable)
    (ghost : x.g.toGhost = y.g.toGhost) (reads : x.g.opRead = y.g.opRead)
    (removed : x.g.removed = y.g.removed) (durable : x.g.delDurable = y.g.delDurable) : x = y := by
  cases x with
  | mk xs xg =>
    cases y with
    | mk ys yg =>
      cases xs; cases ys; cases xg; cases yg; simp_all
end Storage3
namespace Storage35Base
open Storage21 Storage32 Storage3
set_option maxHeartbeats 8000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false

theorem compound_discard_bad10 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (held : (x.s.pages m.p).held = true) :
    (upDiscard c x {x.s with up := tail} m true).g.bad = x.g.bad := by
  simp only [upDiscard,held,Bool.not_true,Bool.false_eq_true,ite_false]
  have pe : partner {x.s with up := tail} m.p = partner x.s m.p := rfl
  simp only [pe]
  let src := (partner x.s m.p).getD m.p
  split
  · simp only [Storage32.commit,Storage3.commit]
    apply commit_named_bad10 (ps := [m.p,src])
    · intro p hp
      have ne : p ≠ m.p ∧ p ≠ src := by simpa using hp
      simp [Storage32.discarded,pages_ackMail,put,ne.1,ne.2,src,Storage32.project,Storage3.project]
    · intro p hp; simpa [src] using hp
    · cases m.cur <;> rfl
    · rfl
  · simp only [Storage32.commit,Storage3.commit]
    apply commit_named_bad10 (ps := [m.p])
    · intro p hp
      have ne : p ≠ m.p := by simpa using hp
      simp [Storage32.discarded,pages_ackMail,put,ne,Storage32.project,Storage3.project]
    · intro p hp; simpa using hp
    · cases m.cur <;> rfl
    · rfl

theorem ordinary_discard_bad10 (c : Config P) (x : Storage31.State P) (sys : Storage21.Sys P)
    (m : Up P) (held : (sys.pages m.p).held = true)
    (pages : sys.pages = x.s.pages) (windows : sys.w = x.s.w) :
    (Storage21.upDiscard c (Storage3.project x) sys m true).g.bad = x.g.bad := by
  simp only [Storage21.upDiscard,held,Bool.not_true,Bool.false_eq_true,ite_false]
  apply commit_named_bad10 (ps := [m.p])
  · intro p hp
    have ne : p ≠ m.p := by simpa using hp
    simp [pages_ackMail,put,ne,pages,Storage3.project]
  · intro p hp; simpa using hp
  · cases m.cur <;> simpa [ackMail,Storage3.project] using windows
  · rfl

theorem factor_restore10 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (src : Fin P) (held : (x.s.pages m.p).held = true) (sourceHeld : (x.s.pages src).held = true)
    (different : src ≠ m.p) (source : (partner x.s m.p).getD m.p = src)
    (deletion : (x.s.pages src).buf = absent) (mutant : c.mutant = "none") :
    Storage32.project (upDiscard c x {x.s with up := tail} m true) =
      Storage31.silentRestore10 c
        (Storage3.lift (Storage32.project x)
          (Storage21.upDiscard c (Storage3.project (Storage32.project x))
            {x.s.toBase.toSys with up := tail} m true)) src := by
  let y := Storage3.lift (Storage32.project x)
    (Storage21.upDiscard c (Storage3.project (Storage32.project x))
      {x.s.toBase.toSys with up := tail} m true)
  have yheld : (y.s.pages src).held = true := by
    simp only [y,Storage3.lift,Storage21.upDiscard,Storage32.project,Storage3.project,held,
      Bool.not_true,Bool.false_eq_true,ite_false,pages_commit,pages_ackMail,put,different]
    exact sourceHeld
  have ybad : y.g.bad = x.g.bad := ordinary_discard_bad10 c (Storage32.project x) _ m held rfl rfl
  have sbad := Storage31.silent_bad10 c y src yheld
  have fullBad := compound_discard_bad10 c x m tail held
  have pe : partner {x.s with up := tail} m.p = partner x.s m.p := rfl
  have rev : (((partner {x.s with up := tail} m.p).getD m.p != m.p) &&
      (({x.s with up := tail}.pages ((partner {x.s with up := tail} m.p).getD m.p)).held || c.mutant == "MRH") &&
      ({x.s with up := tail}.pages ((partner {x.s with up := tail} m.p).getD m.p)).buf == absent) = true := by
    simp [show partner {x.s with up := tail} m.p = partner x.s m.p from rfl,source,different,sourceHeld,deletion]
  apply Storage3.state_eq10
  all_goals first | apply sys_eq10 | apply ghost_eq10 | skip
  all_goals first | exact fullBad.trans (sbad.trans ybad).symm | skip
  all_goals simp only [Storage32.project,upDiscard,held,Bool.not_true,Bool.false_eq_true,ite_false,rev,ite_true,
      Storage32.commit,Storage3.commit,Storage3.project,Storage3.lift,Storage31.silentRestore10,
      Storage21.upDiscard,Storage31.silentMsg10,yheld,
      Storage32.discarded,y,pe,source,deletion,sourceHeld,different,beq_self_eq_true,Bool.true_and,Bool.and_true,ite_true]
  all_goals cases hc : m.cur <;> simp only [hc,Storage21.commit,ackMail,Bool.not_false,Bool.not_true,
    Bool.true_eq_false,Bool.false_eq_true,ite_false,ite_true,put,different,sourceHeld]
  all_goals first | rfl | skip
  all_goals try simp [different,Int.add_assoc]
  all_goals try
    funext q
    by_cases target : q = m.p
    · subst q
      simp [put,different,Ne.symm different,held,sourceHeld,deletion,dirty,Int.add_assoc,union_repeat10]
    · by_cases srcq : q = src
      · subst q
        simp [put,target,different,Ne.symm different,held,sourceHeld,deletion,dirty,Int.add_assoc,union_repeat10]
      · cases cleanq : clean (x.s.pages q) <;>
          simp [put,target,srcq,different,Ne.symm different,held,sourceHeld,deletion,dirty,cleanq,Int.add_assoc,union_repeat10]
end Storage35Base
