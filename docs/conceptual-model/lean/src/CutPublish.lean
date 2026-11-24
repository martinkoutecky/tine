import CutSplice
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)
abbrev Candidate := Graph
abbrev Candidate.graph (c : Candidate) : Graph := c
structure Engine where
  propose : Plan → Option Candidate
structure Done where
  plan : Plan
  candidate : Candidate
  exceptions : List Report
  deriving DecidableEq, Repr
abbrev Outcome := Result Done

def changed (G H : Graph) := H.filter (fun f => !G.contains f)
def p3Check (O : Observations) (G H : Graph) : Bool :=
  (changed G H).all (fun f => decide (O.save f = f))

def candidateCheck (O : Observations) (G : Graph) (old : String) (p : Plan) (c : Candidate) : Option Refusal :=
  match postSafety P D O G c.graph old p with
  | some reason => some reason
  | none =>
    if !(G ++ c.graph).all (validSites O) || !reportsDistinct (exceptionReports P D O G old p) then some .invalidSites
    else if !p1Check P D O G c.graph old p then some .textP1
    else if !p2Check P D O G c.graph old p then some .observationsP2
    else if !p3Check O G c.graph then some .saveP3
    else none

/-- Proof notation for already selected before safety and checked publication. -/
def Accepted (O : Observations) (G : Graph) (old : String) (p : Plan) (c : Candidate) : Prop :=
  preSafety P D O G old p = none ∧ candidateCheck P D O G old p c = none

def publish (O : Observations) (E : Engine) (G : Graph) (old : String) (p : Plan) : Outcome :=
  match E.propose p with
  | none => .refuse .writerFailed
  | some c => match candidateCheck P D O G old p c with
    | some reason => .refuse reason
    | none =>
      let reports := exceptionReports P D O G old p
      .success ⟨p,c,reports⟩

def decide (O : Observations) (E : Engine) (G : Graph) (r : Request) : Outcome :=
  match select P D O G r with
  | .refuse why => .refuse why | .ask q => .ask q | .success p => publish P D O E G r.source p

def decideRequest (O : Observations) (key : String → K) (url : String → Bool)
    (D : Date13.Dates K) (E : Engine) (G : Graph) (r : Request) : Outcome :=
  decide (readingInputs O key url) D O E G r

theorem decision_total (O : Observations) (E : Engine) (G : Graph) (r : Request) :
    ∃ x, decide P D O E G r = x ∧ ∀ y, decide P D O E G r = y → y = x := ⟨_,rfl,fun _ h => h.symm⟩
theorem decision_deterministic {O : Observations} {E : Engine} {G : Graph} {r : Request} {x y : Outcome}
    (hx : decide P D O E G r = x) (hy : decide P D O E G r = y) : x = y := hx.symm.trans hy

theorem accepted_properties {O : Observations} {G : Graph} {old : String} {p : Plan} {c : Candidate}
    (h : Accepted P D O G old p c) :
    (preSafety P D O G old p) = none ∧
    (postSafety P D O G c.graph old p) = none ∧
    (G ++ c.graph).all (validSites O) = true ∧ reportsDistinct (exceptionReports P D O G old p) = true ∧
    p1Check P D O G c.graph old p = true ∧ p2Check P D O G c.graph old p = true ∧ p3Check O G c.graph = true := by
  rcases h with ⟨hb,h⟩
  refine ⟨hb,?_⟩
  unfold candidateCheck at h
  cases ha : postSafety P D O G c.graph old p with
  | some reason => simp only [ha] at h; contradiction
  | none =>
    simp only [ha] at h
    split at h <;> try contradiction
    split at h <;> try contradiction
    split at h <;> try contradiction
    split at h <;> simp_all [Bool.or_eq_true,Bool.not_eq_true]

theorem published_checked {O : Observations} {E : Engine} {G : Graph}
    {old : String} {p : Plan} {d : Done} (h : publish P D O E G old p = .success d) :
    d.plan = p ∧ candidateCheck P D O G old d.plan d.candidate = none ∧
    d.exceptions = exceptionReports P D O G old p := by
  unfold publish at h
  split at h
  · contradiction
  · split at h
    · contradiction
    · next hc =>
      dsimp only at h
      cases h; exact ⟨rfl,hc,rfl⟩

theorem done_checked {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    ∃ p, select P D O G r = .success p ∧ d.plan = p ∧
      Accepted P D O G r.source d.plan d.candidate ∧
      d.exceptions = exceptionReports P D O G r.source p := by
  unfold decide at h
  split at h <;> try contradiction
  next p hp =>
    obtain ⟨he,hc,hr⟩ := published_checked P D h
    exact ⟨p,hp,he,⟨by rw [he]; exact selected_pre_safe P D hp,hc⟩,hr⟩

theorem done_unattended_no_merge {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (ha : r.attended = false) (h : decide P D O E G r = .success d) : d.plan.survivor = none := by
  obtain ⟨p,hp,he,_⟩ := done_checked P D h
  rw [he]; exact unattended_no_merge P D O G r ha hp

theorem decision_unattended_no_question (O : Observations) (E : Engine) (G : Graph)
    (r : Request) (ha : r.attended = false) (q : Question) : decide P D O E G r ≠ .ask q := by
  unfold decide
  cases hs : select P D O G r with
  | refuse why => simp
  | success p =>
    change publish P D O E G r.source p ≠ .ask q
    unfold publish
    split <;> try simp
    split <;> try simp
  | ask q' => exact False.elim ((unattended_no_question P D O G r ha q') hs)

theorem done_stable {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    ∀ f ∈ changed G d.candidate.graph, O.save f = f := by
  obtain ⟨_,_,_,hc,_⟩ := done_checked P D h
  have hp := (accepted_properties P D hc).2.2.2.2.2.2
  simpa only [p3Check,decide_eq_true_eq] using List.all_eq_true.mp hp

theorem done_reading {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    d.candidate.graph.length = (retained G d.plan).length ∧
    ∀ f k, (f,k) ∈ (retained G d.plan).zip d.candidate.graph → pairMatches P D O r.source d.plan f k := by
  obtain ⟨_,_,_,hc,_⟩ := done_checked P D h
  exact p2_sound P D (accepted_properties P D hc).2.2.2.2.2.1


theorem accepted_stable {O : Observations} {G : Graph} {old : String} {p : Plan} {c : Candidate}
    (h : Accepted P D O G old p c) :
    ∀ f ∈ changed G c.graph, O.save f = f := by
  simpa only [p3Check,decide_eq_true_eq] using List.all_eq_true.mp (accepted_properties P D h).2.2.2.2.2.2

theorem accepted_reports_distinct {O : Observations} {G : Graph} {old : String} {p : Plan} {c : Candidate}
    (h : Accepted P D O G old p c) :
    ((exceptionReports P D O G old p).map (fun r => (r.path,r.start,r.stop,r.kind))).Nodup :=
  of_decide_eq_true (accepted_properties P D h).2.2.2.1

theorem accepted_name_safety {O : Observations} {G : Graph} {old : String} {p : Plan} {c : Candidate}
    (h : Accepted P D O G old p c) :
    WF G ∧ WF c.graph ∧ ∀ s ∈ touchedNames P D G old p,
      Date13.conflicted P D G (Date13.target P D G s) = false := by
  have hb : preSafety P D O G old p = none := by exact (accepted_properties P D h).1
  have ha : postSafety P D O G c.graph old p = none := by exact (accepted_properties P D h).2.1
  exact ⟨pre_wf P D hb,post_wf P D (pre_wf P D hb) ha,pre_touched_clean P D hb⟩

end Cut
end CM
