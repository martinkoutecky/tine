import Invariant31
namespace Storage31
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192

theorem inv_initial (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none")
    (realDisk : ∀ p, RealByte c (disk p)) (noUnknown : NoUnknown6 c) :
    Inv c (initWith c disk) := by
  have hr : Storage3.Reachable c disk (Storage3.initWith c disk) := .initial
  exact {
    inactive := inactive_reachable3 c disk _ mutant hr
    protocol := protocol_reachable3 c disk _ mutant hr
    snapshot := snapshot_reachable4 c disk _ mutant hr
    shapes := shapes_reachable4 c disk _ mutant hr
    bounds := bounds_reachable4 c disk _ mutant hr
    clear := clear_reachable4 c disk _ mutant hr
    bytes := bytes_reachable4 c disk _ mutant realDisk hr
    lineage := lineage_reachable4 c disk _ mutant hr
    windowSeen := windowSeen_reachable4 c disk _ mutant realDisk hr
    carrierSeen := carrierSeen_reachable4 c disk _ mutant realDisk hr
    epochs := epochs_reachable6 c disk _ mutant hr
    distinct := distinct_reachable6 c disk _ mutant hr
    active := active_reachable6 c disk _ mutant hr
    fence := fence_reachable6 c disk _ mutant hr
    promiseBytes := promiseBytes_reachable6 c disk _ mutant realDisk hr
    obs := obs_reachable6 c disk _ mutant realDisk hr
    protection := protected_reachable5 c disk _ mutant realDisk hr
    custody := by simp [Custody,NeedsDraft,LivePromise,Storage3.initWith,embed,project,Storage21.init]
    authority := authority_reachable6 c disk _ mutant realDisk noUnknown hr
    C := C_reachable3 c disk _ mutant hr
    B := B_reachable5 c disk _ mutant realDisk hr
    W := Bprime1_reachable3 c disk _ mutant hr
    taken := Bprime2_reachable3 c disk _ mutant hr
    newTags := newTags_reachable c disk _ mutant hr
    vocabulary := vocabulary_reachable3 c disk _ mutant hr
    unheld := unheld_reachable3 c disk _ mutant hr
    G := G_reachable6 c disk _ mutant realDisk noUnknown hr
    trash := trash_reachable c disk _ mutant hr
  }
end Storage31
