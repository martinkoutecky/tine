import PairDiscard11
import Inputs11
import PairPublication11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def PageSafe11 (disk : Int) (a b : Page) : Prop :=
  (disk ≠ absent → (a.held = true → a.buf ≠ absent) → b.held = true → b.buf ≠ absent) ∧
  (a.held = true → a.buf ≠ absent → a.risk = true →
    b.held = true ∧ b.buf ≠ absent ∧ b.risk = true)
theorem pageSafe_refl11 (d : Int) (a : Page) : PageSafe11 d a a := by
  exact ⟨fun _ h held => h held,fun held buf risk => ⟨held,buf,risk⟩⟩
theorem pageSafe_table11 (c : Config P) (a : Page) (d v : Int) (mutant : c.mutant = "none") :
    PageSafe11 d a (table c a d v) := by
  simp only [table,mutant,show ("none" == "MQ") = false from rfl,
    show ("none" == "MLO") = false from rfl,Bool.false_and,Bool.false_eq_true,
    Bool.false_or,ite_false]
  split <;> (try split) <;> (try split)
  all_goals simp_all [PageSafe11,clean]
theorem pageSafe_close11 (c : Config P) (a : Page) (d : Int) (mutant : c.mutant = "none") :
    PageSafe11 d a (if a.held && (clean a || (c.mutant == "MCL" && !a.risk)) then {} else a) := by
  simp only [mutant,show ("none" == "MCL") = false from rfl,Bool.false_and,Bool.or_false]
  split
  · simp_all [PageSafe11,clean]
  · exact pageSafe_refl11 d a
theorem pageSafe_open11 (a : Page) (d v : Int) (ok : Bool) :
    PageSafe11 d a (if a.held || !ok then a else {held := true,buf := d,base := d,obs := d,ver := v}) := by
  split
  · exact pageSafe_refl11 d a
  · simp_all [PageSafe11]
theorem pageSafe_submit11 (c : Config P) (a : Page) (d t bv v : Int)
    (mutant : c.mutant = "none") (text : t ≠ absent) : PageSafe11 d a (submitTo c a t bv v) := by
  simp only [submitTo,mutant,show ("none" == "MIS") = false from rfl,
    show ("none" == "MST") = false from rfl,Bool.false_eq_true,Bool.or_false,ite_false]
  split <;> simp_all [PageSafe11]
theorem pageSafe_risk11 (a : Page) (d : Int) : PageSafe11 d a {a with risk := true} := by
  exact ⟨fun _ h held => h held,fun held buf _ => ⟨held,buf,rfl⟩⟩
theorem pageSafe_reply11 (c : Config P) (a : Page) (d b : Int) (out : String)
    (mutant : c.mutant = "none") (notpub : out ≠ "Published") :
    PageSafe11 d a (onReply c a out b) := by
  simpa [onReply,mutant,notpub] using pageSafe_risk11 a d
theorem pageSafe_raise11 (d : Int) (a b : Page) (h : PageSafe11 d a b) :
    PageSafe11 d a {b with risk := true} := by
  exact ⟨h.1,fun held buf risk => ⟨(h.2 held buf risk).1,(h.2 held buf risk).2.1,rfl⟩⟩
theorem pageSafe_resolve11 (a : Page) (d t base ver : Int) (text : t ≠ absent) :
    PageSafe11 d a {a with buf := t,base := base,typed := true,ver := ver,conflict := a.obs != base,risk := a.risk || a.obs != base} := by
  constructor
  · exact fun _ _ _ => text
  · intro held _ risk
    exact ⟨held,text,by simp [risk]⟩

theorem pageSafe_put11 (disk : Fin P → Int) (a b : Fin P → Page) (p : Fin P) (pg : Page)
    (all : ∀ q, PageSafe11 (disk q) (a q) (b q)) (hp : PageSafe11 (disk p) (a p) pg) :
    ∀ q, PageSafe11 (disk q) (a q) (put b p pg q) := by
  intro q; by_cases eq : q = p
  · subst q; simpa [put] using hp
  · simpa [put,eq] using all q

theorem pairSafety_frame11 (x y : State P) (h : PairSafety10 x)
    (disk : y.s.disk = x.s.disk) (pairs : ∀ r ∈ y.g.renamed, r ∈ x.g.renamed)
    (pages : ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p) (y.s.pages p))
    (gates : ∀ gt ∈ x.s.gates, gt ∈ y.s.gates)
    (job : y.s.job.on = true → x.s.job.on = true ∧ y.s.job.p = x.s.job.p ∧ y.s.job.bytes = x.s.job.bytes) :
    PairSafety10 y := by
  have stable (p : Fin P) (hp : StableSide10 x p) : StableSide10 y p := by
    refine ⟨by simpa [disk] using hp.1,(pages p).1 hp.1 hp.2.1,?_⟩
    intro on eq
    obtain ⟨oldon,p,b⟩ := job on
    rw [b]
    exact hp.2.2 oldon (p.symm.trans eq)
  intro r mem
  rcases h r (pairs r mem) with src | dst | wait
  · exact Or.inl (stable r.1 src)
  · exact Or.inr (Or.inl (stable r.2 dst))
  · right; right
    obtain ⟨dh,db,dr⟩ := (pages r.2).2 wait.2.1 wait.2.2.1 wait.2.2.2.1
    obtain ⟨gt,gm,gs,gd,sm,dm⟩ := wait.2.2.2.2.2.2
    refine ⟨by simpa [disk] using wait.1,dh,db,dr,?_,?_,gt,gates gt gm,gs,gd,sm,dm⟩
    · intro on eq
      obtain ⟨oldon,p,_⟩ := job on
      exact wait.2.2.2.2.1 oldon (p.symm.trans eq)
    · intro on eq
      obtain ⟨oldon,p,b⟩ := job on
      rw [b]
      exact wait.2.2.2.2.2.1 oldon (p.symm.trans eq)

def core11 (x : State P) : Storage21.State P := Storage3.project (Storage32.project (project x))
def coreLift11 (x : State P) (y : Storage21.State P) : State P :=
  lift x (Storage32.lift (project x) (Storage3.lift (Storage32.project (project x)) y))
theorem pairSafety_core11 (x : State P) (y : Storage21.State P) (h : PairSafety10 x)
    (disk : y.s.disk = x.s.disk)
    (pages : ∀ p, PageSafe11 (x.s.disk p) (x.s.pages p) (y.s.pages p))
    (job : y.s.job.on = true → x.s.job.on = true ∧ y.s.job.p = x.s.job.p ∧ y.s.job.bytes = x.s.job.bytes) :
    PairSafety10 (coreLift11 x y) :=
  pairSafety_frame11 x (coreLift11 x y) h disk (fun _ hm => hm) pages (fun _ hm => hm) job

theorem pairSafety_endpoints11 (x y : State P) (h : PairSafety10 x)
    (disk : y.s.disk = x.s.disk) (pairs : ∀ r ∈ y.g.renamed, r ∈ x.g.renamed)
    (pages : ∀ r ∈ y.g.renamed, PageSafe11 (x.s.disk r.1) (x.s.pages r.1) (y.s.pages r.1) ∧
      PageSafe11 (x.s.disk r.2) (x.s.pages r.2) (y.s.pages r.2))
    (gates : ∀ gt ∈ x.s.gates, gt ∈ y.s.gates) (job : y.s.job = x.s.job) : PairSafety10 y := by
  intro r mem
  have ps := pages r mem
  have stable (p : Fin P) (hp : StableSide10 x p) (page : PageSafe11 (x.s.disk p) (x.s.pages p) (y.s.pages p)) :
      StableSide10 y p := by
    exact ⟨by simpa [disk] using hp.1,page.1 hp.1 hp.2.1,by simpa only [job] using hp.2.2⟩
  rcases h r (pairs r mem) with src | dst | wait
  · exact Or.inl (stable r.1 src ps.1)
  · exact Or.inr (Or.inl (stable r.2 dst ps.2))
  · right; right
    obtain ⟨dh,db,dr⟩ := ps.2.2 wait.2.1 wait.2.2.1 wait.2.2.2.1
    obtain ⟨gt,gm,gs,gd,sm,dm⟩ := wait.2.2.2.2.2.2
    exact ⟨by simpa [disk] using wait.1,dh,db,dr,
      by simpa only [job] using wait.2.2.2.2.1,
      by simpa only [job] using wait.2.2.2.2.2.1,gt,gates gt gm,gs,gd,sm,dm⟩
end Storage35
