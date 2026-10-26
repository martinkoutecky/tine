import Guarantee31
namespace Storage31
open Storage21 Storage3
set_option maxHeartbeats 4000000

theorem initial_load_enabled7 (c : Config P) (disk : Fin P → Int) (p : Fin P) :
    Storage31.enabled c (initWith c disk) (.load p) = true := by
  simp [Storage31.enabled,Storage3.initWith,embed,Storage21.init]

theorem initial_load_reachable7 (c : Config P) (disk : Fin P → Int) (p : Fin P) :
    Storage31.Reachable c disk (load c (initWith c disk) p) := by
  exact .advance .initial (.load p) (by simp [Storage31.step,initial_load_enabled7,Storage31.execute])

theorem initial_load_read7 (c : Config P) (disk : Fin P → Int) (p : Fin P) :
    (load c (initWith c disk) p).s.pages p =
      ({held := true,buf := disk p,base := disk p,obs := disk p,ver := 1} : Page) ∧
    clean ((load c (initWith c disk) p).s.pages p) = true ∧
    (load c (initWith c disk) p).g.vc = 1 ∧
    (load c (initWith c disk) p).s.w = (initWith c disk).s.w ∧
    (load c (initWith c disk) p).s.up = [] := by
  simp [load,Storage3.initWith,embed,Storage21.init,Storage3.commit,project,Storage21.commit,put,clean]

theorem load_delete_enabled7 (c : Config P) (disk : Fin P → Int) (p : Fin P)
    (mutant : c.mutant = "none") (present : disk p ≠ absent) :
    Storage31.enabled c (load c (initWith c disk) p) (.opDelete p) = true := by
  simp [Storage31.enabled,Storage3.enabled,Storage3.opFree,opHeld,opClean,opCur,load,
    Storage3.initWith,embed,Storage21.init,nojob,Storage3.commit,project,Storage21.commit,put,clean,mutant,present]

#print axioms initial_load_reachable7
#print axioms load_delete_enabled7
end Storage31
