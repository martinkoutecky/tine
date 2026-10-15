import Bounds9
import Reduction9
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000

theorem bounds_reachable34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    VersionBounds x.s.to32.toBase.toSys x.g.vc :=
  Storage33.bounds_reachable33_9 c disk (project x) mutant (projects_reachable33_9 c disk x mutant hr)

#print axioms bounds_reachable34_9
end Storage34
