import Guarantee35NoApply
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 8000000
set_option linter.unusedSimpArgs false

def batchVer10 (x : State P) (p : Fin P) : Int := x.g.vc+1+p.val
def batchPage10 (x : State P) (values : Fin P → Int) (p : Fin P) : Page :=
  {x.s.pages p with buf := values p,ver := batchVer10 x p,typed := true,risk := true}
def batchSys10 (x : State P) (ps : List (Fin P)) (values : Fin P → Int) : Storage3.Sys P :=
  {x.s with
    pages := (fun p => if p ∈ ps then batchPage10 x values p else x.s.pages p),
    drafts := (fun p => if p ∈ ps then ⟨values p,(x.s.pages p).base,batchVer10 x p⟩ else x.s.drafts p)}
def batchGhost10 (x : State P) (ps : List (Fin P)) (values : Fin P → Int) : Storage3.Ghost P :=
  {x.g with
    vc := x.g.vc+P,mine := ps,
    promise := (fun p => if p ∈ ps then ⟨true,values p,batchVer10 x p,false,0⟩ else x.g.promise p),
    wrote := (fun p => if p ∈ ps then union (x.g.wrote p) [(values p,batchVer10 x p)] else x.g.wrote p),
    opRead := (fun p => if p ∈ ps then union (x.g.opRead p) [(x.s.pages p).base] else x.g.opRead p)}
def batch10 (c : Config P) (x : State P) (ps : List (Fin P)) (values : Fin P → Int) : State P :=
  Storage3.commit c x (batchSys10 x ps values) (batchGhost10 x ps values) "opApply"

theorem batch_auth_mono10 (x : State P) (ps : List (Fin P)) (values : Fin P → Int) :
    AuthMono (shadow6 x).g (shadowGhost6 (batchGhost10 x ps values)) := by
  apply shadow_auth_mono6
  · intro _ _ h; exact h
  · intro p w hw; by_cases mem : p ∈ ps <;> simp [batchGhost10,mem,mem_union_general,hw]
  · intro p b hb; by_cases mem : p ∈ ps <;> simp [batchGhost10,mem,mem_union_general,hb]

theorem batch_inv10 (c : Config P) (x : State P) (ps : List (Fin P)) (values : Fin P → Int)
    (h : Inv c x) (alive : x.s.alive = true)
    (held : ∀ p ∈ ps,(x.s.pages p).held = true)
    (cleaned : ∀ p ∈ ps,clean (x.s.pages p) = true)
    (real : ∀ p ∈ ps,RealByte c (values p))
    (free : x.s.job.on = true → x.s.job.p ∉ ps) : Inv c (batch10 c x ps values) := by
  let ns := batchSys10 x ps values
  let ng := batchGhost10 x ps values
  have grow : ∀ p,(x.s.pages p).held = true → (ns.pages p).held = true := by
    intro p hp; by_cases mem : p ∈ ps <;> simp [ns,batchSys10,batchPage10,mem,hp]
  have pgver : ∀ p,(ns.pages p).ver ≤ ng.vc := by
    intro p; by_cases mem : p ∈ ps
    · have lt := p.isLt; simp [ns,ng,batchSys10,batchGhost10,batchPage10,batchVer10,mem]; omega
    · have old := h.bounds.pages p; simp [ns,ng,batchSys10,batchGhost10,mem]; omega
  have prebytes : ByteDomain c ns.toSys := by
    refine ⟨h.bytes.disk,h.bytes.stable,?_,?_,h.bytes.job,h.bytes.w,h.bytes.mb,h.bytes.up⟩
    · intro p; by_cases mem : p ∈ ps
      · simpa [ns,batchSys10,batchPage10,mem] using real p mem
      · simpa [ns,batchSys10,mem] using h.bytes.pages p
    · intro p; by_cases mem : p ∈ ps
      · right; simpa [ns,batchSys10,mem] using real p mem
      · simpa [ns,batchSys10,mem] using h.bytes.drafts p
  have prelineage : PromiseLineage ⟨ns.toSys,ng.toGhost⟩ := by
    refine ⟨?_,?_,?_⟩
    · intro p; by_cases mem : p ∈ ps
      · have lt := p.isLt; simp [ng,batchGhost10,batchVer10,mem]; omega
      · have old := h.lineage.1 p
        change (x.g.promise p).ver ≤ x.g.vc at old
        simp [ng,batchGhost10,mem]; omega
    · intro p hp; by_cases mem : p ∈ ps
      · simp [ns,ng,batchSys10,batchGhost10,batchPage10,mem,EqualLineage]
      · simpa [ns,ng,batchSys10,batchGhost10,mem,project] using h.lineage.2.1 p (by simpa [ns,batchSys10,mem,project] using hp)
    · intro p hc; by_cases mem : p ∈ ps
      · simp [ns,batchSys10,batchPage10,mem,clean] at hc
      · simpa [ns,ng,batchSys10,batchGhost10,mem,project] using h.lineage.2.2 p (by simpa [ns,batchSys10,mem,project] using hc)
  have bad : (batch10 c x ps values).g.bad = x.g.bad := by
    apply commit_named_bad10 (x := project x) (ps := ps)
    · intro p mem; simp [batchSys10,mem,project]
    · intro p mem; exact mem
    · rfl
    · rfl
  have evolution : PageEvolution (shadow6 x) ns.toSys := by
    intro p hp; by_cases mem : p ∈ ps
    · right; simp [ns,batchSys10,batchPage10,batchVer10,mem,shadow6]; omega
    · left
      refine ⟨?_,?_,?_⟩
      · simpa [ns,batchSys10,mem,shadow6] using hp
      · simp [ns,batchSys10,mem,shadow6]
      · simp [ns,batchSys10,mem,shadow6]
  have mono := batch_auth_mono10 x ps values
  refine {
    inactive := by simp [InactivePages,batch10,Storage3.commit,project,Storage21.commit,batchSys10,alive]
    protocol := ?_
    snapshot := ?_
    shapes := ?_
    bounds := ?_
    clear := ?_
    bytes := bytes_commit c (project x) ns.toSys ng.toGhost "opApply" prebytes
    lineage := lineage_commit c (project x) ns.toSys ng.toGhost "opApply" prelineage pgver
    windowSeen := windowSeen_commit c (project x) ns.toSys ng.toGhost "opApply" h.bytes.w
    carrierSeen := ?_
    epochs := ?_
    distinct := h.distinct
    active := ?_
    fence := ?_
    promiseBytes := ?_
    obs := ?_
    protection := ?_
    custody := ?_
    authority := ?_
    C := ?_
    B := ?_
    W := ?_
    taken := ?_
    newTags := by simpa [NewTagsClean,project,bad] using h.newTags
    vocabulary := by intro t ht; exact h.vocabulary t (bad ▸ ht)
    unheld := by rw [bad]; exact h.unheld
    G := by simpa [GOnly,project,bad] using h.G
    trash := h.trash
  }
  · refine ⟨h.protocol.kinds,?_,?_,h.protocol.carriage⟩
    · simp [InactiveClient,batch10,Storage3.commit,project,Storage21.commit,batchSys10,alive]
    · apply lifetime_commit_grow c (project x) ns.toSys ng.toGhost "opApply" h.protocol.life
      · exact alive
      · rfl
      · rfl
      · rfl
      · exact grow
  · intro on
    have mem := free on
    simpa [batch10,Storage3.commit,project,Storage21.commit,batchSys10,mem] using h.snapshot on
  · change Shapes ns.toSys
    intro p; by_cases mem : p ∈ ps
    · simp [ns,batchSys10,batchPage10,PageShape,mem,held p mem]
    · simpa [ns,batchSys10,mem] using h.shapes p
  · apply bounds_commit
    have hg := bounds_mono x.s.toSys x.g.vc ng.vc h.bounds (by simp [ng,batchGhost10]; omega)
    exact ⟨hg.nonnegative,pgver,hg.w,hg.mb,hg.up⟩
  · intro on
    have mem := free on
    simpa [batch10,Storage3.commit,project,Storage21.commit,batchSys10,mem] using h.clear on
  · apply carrierSeen_frame (project x) (project (batch10 c x ps values)) h.carrierSeen rfl
    exact seen_commit c (project x) _ _ "opApply"
  · apply epochs_commit
    refine ⟨h.epochs.ext,?_,h.epochs.job⟩
    intro p; by_cases mem : p ∈ ps
    · simpa [batchGhost10,mem] using h.epochs.ext p
    · simpa [batchGhost10,mem] using h.epochs.promise p
  · apply active_commit
    intro p hp on hv; by_cases mem : p ∈ ps
    · simp [batchSys10,batchGhost10,batchPage10,mem]
    · simpa [batchSys10,batchGhost10,mem] using h.active p
        (by simpa [batchSys10,mem] using hp) (by simpa [batchGhost10,mem] using on)
        (by simpa [batchSys10,batchGhost10,mem] using hv)
  · apply fence_commit
    intro on phase
    have old := h.fence on phase
    have mem := free on
    simpa [PublicationFence,batchSys10,batchGhost10,mem] using old
  · apply promiseBytes_commit
    · intro p on; by_cases mem : p ∈ ps
      · simpa [batchGhost10,mem] using real p mem
      · simpa [batchGhost10,mem] using h.promiseBytes p (by simpa [batchGhost10,mem] using on)
    · exact prebytes.pages
  · apply obsOnly_commit (g := ng.toGhost) (n := "opApply")
    refine ⟨h.obs.1,h.obs.2.1,?_,h.obs.2.2.2.1,h.obs.2.2.2.2⟩
    intro p; by_cases mem : p ∈ ps <;> simpa [batchSys10,batchPage10,mem] using h.obs.2.2.1 p
  · apply protected_commit5
    intro p hp ht; by_cases mem : p ∈ ps
    · right; left
      simp [batchSys10,batchGhost10,batchPage10,mem]
    · simpa [batchSys10,batchGhost10,batchPage10,mem,TypedProtected5,Blind5,project] using h.protection p
        (by simpa [batchSys10,mem,project] using hp) (by simpa [batchSys10,mem,project] using ht)
  · apply custody_commit
    · intro p need; by_cases mem : p ∈ ps
      · exact ⟨by simp [batchSys10,batchGhost10,mem],by intro _; simp [batchSys10,batchPage10,mem,held p mem]⟩
      · have oldneed : NeedsDraft c (project x) p := by
          simpa [NeedsDraft,LivePromise,Covers,batchSys10,batchGhost10,mem,project] using need
        simpa [batchSys10,batchGhost10,mem,project] using h.custody p oldneed
    · apply postPromises_noCleaning
      intro p hp hd; by_cases mem : p ∈ ps
      · simp [batchSys10,batchPage10,mem,dirty,clean]
      · simpa [batchSys10,mem,project] using hd
  · refine ⟨?_,?_,?_⟩
    · apply shadow_Gcommit6
      rcases h.authority.structural with ⟨_,pg,dr,jb,win,checked⟩
      have admissible : ∀ p ∈ ps,Admissible (shadowGhost6 ng) p (ns.pages p).buf (ns.pages p).base := by
        intro p mem; right; right; right
        refine ⟨0,?_⟩
        simp [ns,ng,batchSys10,batchPage10,batchGhost10,shadowGhost6,mem,mem_union_general]
      refine ⟨by simp [shadowGhost6],?_,?_,?_,?_,checked⟩
      · intro p hp; by_cases mem : p ∈ ps
        · exact admissible p mem
        · simpa [ns,batchSys10,mem,shadow6] using admissible_mono mono (pg p (by simpa [ns,batchSys10,mem,shadow6] using hp))
      · intro p present; by_cases mem : p ∈ ps
        · simpa [ns,batchSys10,mem,draftEntry,held p mem,batchPage10] using
            admissible_draftEntry (shadowGhost6 ng) p (ns.pages p) (fun _ => admissible p mem)
              (by simpa [ns,batchSys10,mem,draftEntry,held p mem,batchPage10] using present)
        · simpa [ns,batchSys10,mem,shadow6] using admissible_mono mono (dr p (by simpa [ns,batchSys10,mem,shadow6] using present))
      · intro on; exact admissible_mono mono (jb on)
      · intro on phase; exact mono _ _ (win on phase)
    · apply shadow_anchors_commit6
      apply anchors_transport (shadow6 x) ns.toSys (shadowGhost6 ng) h.authority.anchors h.bounds evolution
      · rfl
      · rfl
      · intro m hm; exact hm
      · exact mono
    · apply shadow_resolution_commit6
      intro m hm ro; exact mono _ _ (h.authority.resolution m hm ro)
  · change CInv c (Storage21.commit c (project x) ns.toSys ng.toGhost "opApply")
    rw [C_commit]
    exact h.C
  · change "B-unsaved-replaced" ∉ (batch10 c x ps values).g.bad
    rw [bad]; exact h.B
  · change "B'-window-replaced" ∉ (batch10 c x ps values).g.bad
    rw [bad]; exact h.W
  · change "B'-submit-not-taken" ∉ (batch10 c x ps values).g.bad
    rw [bad]; exact h.taken
end Storage31
