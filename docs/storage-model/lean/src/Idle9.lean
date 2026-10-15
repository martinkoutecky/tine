import Storage34
import CoreAccepted21
namespace Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
-- Exact fixed points of the ordinary commit on unchanged page/window state.
def IdleClosed (c : Config P) (x : State P) : Prop  := 
  x.g.mine = [] ∧ ∀ p, (x.s.w p).on = true →
    ((x.s.w p).text == absent || c.texts.contains (x.s.w p).text) = true →
    (x.s.w p).text ∈ x.g.seen p

theorem idle_union9 [BEq α] [LawfulBEq α] (xs ys : List α) (h : ∀ y ∈ ys, y ∈ xs) :
    union xs ys = xs  :=  by
  induction ys generalizing xs with
  | nil => rfl
  | cons y ys ih =>
    have hy : xs.contains y = true  :=  by simpa using h y (by simp)
    simp only [union,List.foldl_cons,hy,ite_true]
    exact ih xs (by intro y hy; exact h y (by simp [hy]))

theorem idle_commit9 (c : Config P) (x : State P) (h : IdleClosed c x) (name : String) :
    commit c x x.s x.g name = x  :=  by
  have np : (fun p => if !(x.g.mine.contains p) && (x.s.pages p).held && dirty (x.s.pages p) &&
      (x.s.pages p).held && clean (x.s.pages p) && (x.s.pages p).buf == (x.s.pages p).buf then
      ack (x.g.promise p) (x.s.pages p).buf (x.s.pages p).ver false 0 else x.g.promise p) = x.g.promise  :=  by
    funext p
    cases hc : clean (x.s.pages p) <;> simp [dirty,hc]
  have mb : (fun p => if x.s.alive && ((x.s.w p).on || (x.s.w p).sent) &&
      ((x.s.pages p).ver != (x.s.pages p).ver || (x.s.pages p).buf != (x.s.pages p).buf ||
       (x.s.pages p).obs != (x.s.pages p).obs || (x.s.pages p).conflict != (x.s.pages p).conflict ||
       (x.s.pages p).held != (x.s.pages p).held) then
       (⟨true,(x.s.pages p).held,(x.s.pages p).ver,(x.s.pages p).buf,(x.s.pages p).obs,
         (x.s.pages p).conflict,if (x.s.mb p).on then (x.s.mb p).ack else -1,
         (x.s.mb p).on && (x.s.mb p).took⟩ : Mail) else x.s.mb p) = x.s.mb  :=  by
    funext p; simp
  have seen : (fun p => union (x.g.seen p) ([(x.s.w p).text].filter fun t =>
      (x.s.w p).on && (t == absent || c.texts.contains t))) = x.g.seen  :=  by
    funext p
    apply idle_union9
    intro t ht
    have hm  :=  List.mem_filter.mp ht
    have eq : t = (x.s.w p).text  :=  by simpa using hm.1
    subst t
    simp only [Bool.and_eq_true] at hm
    exact h.2 p hm.2.1 hm.2.2
  have kb : (ids P).all (fun p => keptB x p x.s x.g.promise x.g.mine name) = true  :=  by
    apply List.all_eq_true.mpr
    intro p _; cases hh : (x.s.pages p).held <;> simp [keptB,hh]
  have kw : (ids P).all (fun p => keptW x p x.s x.g.owed name) = true  :=  by
    apply List.all_eq_true.mpr
    intro p _; cases hh : (x.s.w p).on <;> simp [keptW,hh]
  have unil (ys : List String) : union ys [] = ys := rfl
  simp only [commit,np,mb,seen,kb,kw,Bool.not_true,tags,Bool.false_eq_true,ite_false,unil]
  cases x with | mk s g => cases g; simp_all [IdleClosed]

theorem idle_closed_commit9 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (name : String) :
    IdleClosed c (commit c x s g name)  :=  by
  refine ⟨rfl,?_⟩
  intro p on real
  change (s.w p).text ∈ union (g.seen p) ([(s.w p).text].filter fun t => (s.w p).on && (t == absent || c.texts.contains t))
  apply (mem_union_general _ _ _).mpr
  right
  have on0 : (s.w p).on = true := on
  have real0 : ((s.w p).text == absent || c.texts.contains (s.w p).text) = true := real
  apply List.mem_filter.mpr
  refine ⟨by simp,?_⟩
  change ((s.w p).on && ((s.w p).text == absent || c.texts.contains (s.w p).text)) = true
  rw [on0,real0]
  rfl
end Storage21
