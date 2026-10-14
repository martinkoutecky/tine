import CoreInactive21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem property_put [DecidableEq α] (m : α → β) (p : α) (b : β) (pr : β → Prop)
    (hm : ∀ q, pr (m q)) (hb : pr b) : ∀ q, pr (put m p b q) := by
  intro q; unfold put; split <;> simp_all

def OnlyCleanReplacement (x : State P) (s : Sys P) : Prop := ∀ p,
  ((x.s.pages p).held && (!(s.pages p).held || (s.pages p).buf != (x.s.pages p).buf)) = true →
    clean (x.s.pages p) = true

theorem B_commit_clean (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B-unsaved-replaced" ∉ g.bad) (hc : CleanSafe x)
    (hp : g.promise = x.g.promise) (hr : OnlyCleanReplacement x s) : BInv (commit c x s g n) := by
  apply B_commit c x s g n hb
  intro p
  by_cases replaced : ((x.s.pages p).held && (!(s.pages p).held || (s.pages p).buf != (x.s.pages p).buf)) = true
  · have cl := hr p replaced
    have ht : (x.s.pages p).typed = false := by
      have hcl : (!(x.s.pages p).typed && (x.s.pages p).buf == (x.s.pages p).base && !(x.s.pages p).risk) = true := cl
      simp only [Bool.and_eq_true,Bool.not_eq_true] at hcl
      simpa using hcl.1.1
    have hpost : PostPromises x s g p = x.g.promise p := by simp [PostPromises,dirty,cl,hp]
    rcases hc p cl with same | ⟨ver,bytes⟩
    · simp [keptB,replaced,hpost,ht,same]
    · simp [keptB,replaced,hpost,ver,bytes]
  · simp [keptB,replaced]

theorem table_keeps_unclean (c : Config P) (mutant : c.mutant = "none") (pg : Page) (d v : Int)
    (hc : clean pg ≠ true) : (table c pg d v).buf = pg.buf := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  have he : clean {pg with obs := d} ≠ true := hc
  simp only [he,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> rfl

theorem onlyClean_table (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P) (d v : Int) :
    OnlyCleanReplacement x {x.s with pages := put x.s.pages p (table c (x.s.pages p) d v)} := by
  intro q replaced
  by_cases he : q = p
  · subst q
    by_cases cl : clean (x.s.pages p) = true
    · exact cl
    · have hb := table_keeps_unclean c mutant (x.s.pages p) d v cl
      simp [put,table_held,hb] at replaced
  · simp [put,he] at replaced

theorem B_observe (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (hx : BInv x) (hc : CleanSafe x) : BInv (observe c x p) := by
  apply B_commit_clean
  · exact hx
  · exact hc
  · rfl
  · exact onlyClean_table c mutant x p _ _

theorem B_check (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (hx : BInv x) (hc : CleanSafe x) : BInv (check c x) := by
  simp only [check]; split
  · apply B_commit_clean
    · exact hx
    · exact hc
    · rfl
    · exact onlyClean_table c mutant x x.s.job.p _ _
  · exact B_commit_noReplacement c x _ _ _ hx (noReplacement_refl x)

theorem B_upClose (c : Config P) (mutant : c.mutant = "none") (x : State P)
    (m : Up P) (tail : List (Up P)) (hx : BInv x) (hc : CleanSafe x) :
    BInv (upClose c x {x.s with up := tail} m) := by
  apply B_commit_clean
  · exact hx
  · exact hc
  · rfl
  · simp only [mutant]
    split
    · have cl : clean (x.s.pages m.p) = true := by
        have hgo : ((x.s.pages m.p).held && (clean (x.s.pages m.p) || "none" == "MCL" && !(x.s.pages m.p).risk)) = true := by assumption
        simp only [Bool.and_eq_true] at hgo
        simpa using hgo.2
      intro p hp
      by_cases he : p = m.p
      · subst p; exact cl
      · simp [put,he] at hp
    · intro p hp; simp at hp
#print axioms B_observe
#print axioms B_check
#print axioms B_upClose
end Storage21
