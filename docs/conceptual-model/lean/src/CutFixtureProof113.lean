import CutFixtureProof112
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def fixtureFiles : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩,⟨"pages/B.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩,⟨"pages/A.md",.pages,.md,"- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩,⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩,⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩,⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩,⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩,⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩,⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩,⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩,⟨"pages/B.md",.pages,.md,"- target"⟩,⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- #Old\n"⟩,⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩,⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩,⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩,⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩,⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩,⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩,⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/New.md",.pages,.md,"- target\n"⟩,⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩,⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩,⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩,⟨"pages/old.md",.pages,.md,"- notes\n"⟩,⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩,⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩,⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩,⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩,⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩,⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩,⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩,⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩,⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩,⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩,⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩,⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩,⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩,⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩,⟨"pages/2026.md",.pages,.md,"- notes\n"⟩,⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩,⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩,⟨"pages/B.md",.pages,.md,"- notes\n"⟩,⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩,⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩,⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩,⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩,⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩,⟨"pages/A.md",.pages,.md,"\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩,⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩,⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩,⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩,⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩,⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩,⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩,⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩,⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩,⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩,⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩,⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩,⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩,⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩,⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩,⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩,⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩,⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩,⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩,⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩,⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩,⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩,⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩]
theorem fixture_reader_0 : validSites o ⟨"pages/A.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/A.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_1 : validSites o ⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩ = true := geometry_law_1
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[A]]\n") && titlePartsLaw (nativeRead .md "- [[A]]\n") = true
  rw [hg, native_read_1, fragment_law_1, title_parts_law_1]
  rfl
theorem fixture_reader_2 : validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_3 : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ = true := geometry_law_2
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[B]]\n") && titlePartsLaw (nativeRead .md "- [[B]]\n") = true
  rw [hg, native_read_2, fragment_law_2, title_parts_law_2]
  rfl
theorem fixture_reader_4 : validSites o ⟨"pages/A.md",.pages,.md,"- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"- source\n"⟩ = true := geometry_law_3
  change validSites o ⟨"pages/A.md",.pages,.md,"- source\n"⟩ && fragmentLaw .md (nativeRead .md "- source\n") && titlePartsLaw (nativeRead .md "- source\n") = true
  rw [hg, native_read_3, fragment_law_3, title_parts_law_3]
  rfl
theorem fixture_reader_5 : validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ = true := geometry_law_4
  change validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n") && titlePartsLaw (nativeRead .md "- target\n") = true
  rw [hg, native_read_4, fragment_law_4, title_parts_law_4]
  rfl
theorem fixture_reader_6 : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ = true := geometry_law_5
  change validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n- source\n") && titlePartsLaw (nativeRead .md "- target\n- source\n") = true
  rw [hg, native_read_5, fragment_law_5, title_parts_law_5]
  rfl
theorem fixture_reader_7 : validSites o ⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩ = true := geometry_law_6
  change validSites o ⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩ && fragmentLaw .md (nativeRead .md "clash:: source\njoin:: value\n- s\n") && titlePartsLaw (nativeRead .md "clash:: source\njoin:: value\n- s\n") = true
  rw [hg, native_read_6, fragment_law_6, title_parts_law_6]
  rfl
theorem fixture_reader_8 : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ = true := geometry_law_7
  change validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ && fragmentLaw .md (nativeRead .md "clash:: target\n- t\n") && titlePartsLaw (nativeRead .md "clash:: target\n- t\n") = true
  rw [hg, native_read_7, fragment_law_7, title_parts_law_7]
  rfl
theorem fixture_reader_9 : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩ = true := geometry_law_8
  change validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩ && fragmentLaw .md (nativeRead .md "clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n") && titlePartsLaw (nativeRead .md "clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n") = true
  rw [hg, native_read_8, fragment_law_8, title_parts_law_8]
  rfl
theorem fixture_reader_10 : validSites o ⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩ = true := geometry_law_9
  change validSites o ⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "#+title: A\n* source\n") && titlePartsLaw (nativeRead .org "#+title: A\n* source\n") = true
  rw [hg, native_read_9, fragment_law_9, title_parts_law_9]
  rfl
theorem fixture_reader_11 : validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩ = true := geometry_law_10
  change validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩ && fragmentLaw .org (nativeRead .org "#+title: B\n* target\n") && titlePartsLaw (nativeRead .org "#+title: B\n* target\n") = true
  rw [hg, native_read_10, fragment_law_10, title_parts_law_10]
  rfl
theorem fixture_reader_12 : validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩ = true := geometry_law_11
  change validSites o ⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n") && titlePartsLaw (nativeRead .org "#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n") = true
  rw [hg, native_read_11, fragment_law_11, title_parts_law_11]
  rfl
theorem fixture_reader_13 : validSites o ⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩ = true := geometry_law_12
  change validSites o ⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩ && fragmentLaw .md (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n") && titlePartsLaw (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n") = true
  rw [hg, native_read_12, fragment_law_12, title_parts_law_12]
  rfl
theorem fixture_reader_14 : validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ && fragmentLaw .org (o.content ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩) && titlePartsLaw (o.content ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ = true := geometry_law_13
  change validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ && fragmentLaw .org (nativeRead .org "* h :Old:\n") && titlePartsLaw (nativeRead .org "* h :Old:\n") = true
  rw [hg, native_read_13, fragment_law_13, title_parts_law_13]
  rfl
theorem fixture_reader_15 : validSites o ⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩ && fragmentLaw .org (o.content ⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩) && titlePartsLaw (o.content ⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩) = true := by
  have hg : validSites o ⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩ = true := geometry_law_14
  change validSites o ⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩ && fragmentLaw .org (nativeRead .org "<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n") && titlePartsLaw (nativeRead .org "<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n") = true
  rw [hg, native_read_14, fragment_law_14, title_parts_law_14]
  rfl
theorem fixture_reader_16 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_17 : validSites o ⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩) && titlePartsLaw (o.content ⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩ = true := geometry_law_15
  change validSites o ⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩ && fragmentLaw .md (nativeRead .md "- #Old\n") && titlePartsLaw (nativeRead .md "- #Old\n") = true
  rw [hg, native_read_15, fragment_law_15, title_parts_law_15]
  rfl
theorem fixture_reader_18 : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_19 : validSites o ⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩ = true := geometry_law_16
  change validSites o ⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩ && fragmentLaw .md (nativeRead .md "- #[[Two Words]]\n") && titlePartsLaw (nativeRead .md "- #[[Two Words]]\n") = true
  rw [hg, native_read_16, fragment_law_16, title_parts_law_16]
  rfl
theorem fixture_reader_20 : validSites o ⟨"pages/B.md",.pages,.md,"- target"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target"⟩ = true := geometry_law_17
  change validSites o ⟨"pages/B.md",.pages,.md,"- target"⟩ && fragmentLaw .md (nativeRead .md "- target") && titlePartsLaw (nativeRead .md "- target") = true
  rw [hg, native_read_17, fragment_law_17, title_parts_law_17]
  rfl
theorem fixture_reader_21 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩ = true := geometry_law_18
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: C#\n- source\n") && titlePartsLaw (nativeRead .md "alias:: C#\n- source\n") = true
  rw [hg, native_read_18, fragment_law_18, title_parts_law_18]
  rfl
theorem fixture_reader_22 : validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩ = true := geometry_law_19
  change validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: C\\#\n- target\n") && titlePartsLaw (nativeRead .md "alias:: C\\#\n- target\n") = true
  rw [hg, native_read_19, fragment_law_19, title_parts_law_19]
  rfl
theorem fixture_reader_23 : validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩ = true := geometry_law_20
  change validSites o ⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: C\\#\n- target\n- source\n") && titlePartsLaw (nativeRead .md "alias:: C\\#\n- target\n- source\n") = true
  rw [hg, native_read_20, fragment_law_20, title_parts_law_20]
  rfl
theorem fixture_reader_24 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_25 : validSites o ⟨"pages/R.md",.pages,.md,"- #Old\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- #Old\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- #Old\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- #Old\n"⟩ = true := geometry_law_15
  change validSites o ⟨"pages/R.md",.pages,.md,"- #Old\n"⟩ && fragmentLaw .md (nativeRead .md "- #Old\n") && titlePartsLaw (nativeRead .md "- #Old\n") = true
  rw [hg, native_read_15, fragment_law_15, title_parts_law_15]
  rfl
theorem fixture_reader_26 : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_27 : validSites o ⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩ = true := geometry_law_16
  change validSites o ⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩ && fragmentLaw .md (nativeRead .md "- #[[Two Words]]\n") && titlePartsLaw (nativeRead .md "- #[[Two Words]]\n") = true
  rw [hg, native_read_16, fragment_law_16, title_parts_law_16]
  rfl
theorem fixture_reader_28 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_29 : validSites o ⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩ = true := geometry_law_21
  change validSites o ⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩ && fragmentLaw .md (nativeRead .md "- Old:: value\n- Old:: another\n") && titlePartsLaw (nativeRead .md "- Old:: value\n- Old:: another\n") = true
  rw [hg, native_read_21, fragment_law_21, title_parts_law_21]
  rfl
theorem fixture_reader_30 : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_31 : validSites o ⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩ = true := geometry_law_22
  change validSites o ⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩ && fragmentLaw .md (nativeRead .md "- two-words:: value\n- two-words:: another\n") && titlePartsLaw (nativeRead .md "- two-words:: value\n- two-words:: another\n") = true
  rw [hg, native_read_22, fragment_law_22, title_parts_law_22]
  rfl
theorem fixture_reader_32 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_33 : validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_34 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_35 : validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_36 : validSites o ⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩ = true := geometry_law_24
  change validSites o ⟨"pages/1.md",.pages,.md,"title:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "title:: X\n- source\n") && titlePartsLaw (nativeRead .md "title:: X\n- source\n") = true
  rw [hg, native_read_24, fragment_law_24, title_parts_law_24]
  rfl
theorem fixture_reader_37 : validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩ = true := geometry_law_25
  change validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "title:: X\n- target\n") && titlePartsLaw (nativeRead .md "title:: X\n- target\n") = true
  rw [hg, native_read_25, fragment_law_25, title_parts_law_25]
  rfl
theorem fixture_reader_38 : validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩ = true := geometry_law_26
  change validSites o ⟨"pages/2.md",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "title:: X\n- target\n- \n  title:: X\n- source\n") && titlePartsLaw (nativeRead .md "title:: X\n- target\n- \n  title:: X\n- source\n") = true
  rw [hg, native_read_26, fragment_law_26, title_parts_law_26]
  rfl
theorem fixture_reader_39 : validSites o ⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩ = true := geometry_law_27
  change validSites o ⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "x:: one\nx:: two\n- source\n") && titlePartsLaw (nativeRead .md "x:: one\nx:: two\n- source\n") = true
  rw [hg, native_read_27, fragment_law_27, title_parts_law_27]
  rfl
theorem fixture_reader_40 : validSites o ⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩ = true := geometry_law_28
  change validSites o ⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "x:: one\n- target\n- \n  x:: two\n- source\n") && titlePartsLaw (nativeRead .md "x:: one\n- target\n- \n  x:: two\n- source\n") = true
  rw [hg, native_read_28, fragment_law_28, title_parts_law_28]
  rfl
theorem fixture_reader_41 : validSites o ⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩ && fragmentLaw .md (o.content ⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩) && titlePartsLaw (o.content ⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩) = true := by
  have hg : validSites o ⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩ = true := geometry_law_29
  change validSites o ⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: A\n- owner\n") && titlePartsLaw (nativeRead .md "alias:: A\n- owner\n") = true
  rw [hg, native_read_29, fragment_law_29, title_parts_law_29]
  rfl
theorem fixture_reader_42 : validSites o ⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩ && fragmentLaw .md (o.content ⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩) && titlePartsLaw (o.content ⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩) = true := by
  have hg : validSites o ⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩ = true := geometry_law_30
  change validSites o ⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: B\n- owner\n") && titlePartsLaw (nativeRead .md "alias:: B\n- owner\n") = true
  rw [hg, native_read_30, fragment_law_30, title_parts_law_30]
  rfl
theorem fixture_reader_43 : validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩ = true := geometry_law_31
  change validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "title:: 2026-11-02\n- target\n") && titlePartsLaw (nativeRead .md "title:: 2026-11-02\n- target\n") = true
  rw [hg, native_read_31, fragment_law_31, title_parts_law_31]
  rfl
theorem fixture_reader_44 : validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩ = true := geometry_law_32
  change validSites o ⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "title:: 2026-11-02\n- target\n- source\n") && titlePartsLaw (nativeRead .md "title:: 2026-11-02\n- target\n- source\n") = true
  rw [hg, native_read_32, fragment_law_32, title_parts_law_32]
  rfl
theorem fixture_reader_45 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_46 : validSites o ⟨"pages/New.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New.md",.pages,.md,"- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/New.md",.pages,.md,"- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New.md",.pages,.md,"- target\n"⟩ = true := geometry_law_4
  change validSites o ⟨"pages/New.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n") && titlePartsLaw (nativeRead .md "- target\n") = true
  rw [hg, native_read_4, fragment_law_4, title_parts_law_4]
  rfl
theorem fixture_reader_47 : validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ = true := geometry_law_33
  change validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: Old/[[New]] [[X]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: Old/[[New]] [[X]]\n- child\n") = true
  rw [hg, native_read_33, fragment_law_33, title_parts_law_33]
  rfl
theorem fixture_reader_48 : validSites o ⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩ = true := geometry_law_34
  change validSites o ⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n- notes\n") && titlePartsLaw (nativeRead .md "- target\n- notes\n") = true
  rw [hg, native_read_34, fragment_law_34, title_parts_law_34]
  rfl
theorem fixture_reader_49 : validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ = true := geometry_law_35
  change validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: New/[[New]] [[X]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: New/[[New]] [[X]]\n- child\n") = true
  rw [hg, native_read_35, fragment_law_35, title_parts_law_35]
  rfl
theorem fixture_reader_50 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_51 : validSites o ⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ = true := geometry_law_36
  change validSites o ⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: Old/[[Old]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: Old/[[Old]]\n- child\n") = true
  rw [hg, native_read_36, fragment_law_36, title_parts_law_36]
  rfl
theorem fixture_reader_52 : validSites o ⟨"pages/old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_53 : validSites o ⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩ = true := geometry_law_37
  change validSites o ⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: old/[[old]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: old/[[old]]\n- child\n") = true
  rw [hg, native_read_37, fragment_law_37, title_parts_law_37]
  rfl
theorem fixture_reader_54 : validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩ = true := geometry_law_38
  change validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "alias::\n- target\n") && titlePartsLaw (nativeRead .md "alias::\n- target\n") = true
  rw [hg, native_read_38, fragment_law_38, title_parts_law_38]
  rfl
theorem fixture_reader_55 : validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩ = true := geometry_law_39
  change validSites o ⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias::\n- target\n- source\n") && titlePartsLaw (nativeRead .md "alias::\n- target\n- source\n") = true
  rw [hg, native_read_39, fragment_law_39, title_parts_law_39]
  rfl
theorem fixture_reader_56 : validSites o ⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩ = true := geometry_law_40
  change validSites o ⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "source prose\n\n- source\n") && titlePartsLaw (nativeRead .md "source prose\n\n- source\n") = true
  rw [hg, native_read_40, fragment_law_40, title_parts_law_40]
  rfl
theorem fixture_reader_57 : validSites o ⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩ = true := geometry_law_41
  change validSites o ⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "target prose\n\n- target\n") && titlePartsLaw (nativeRead .md "target prose\n\n- target\n") = true
  rw [hg, native_read_41, fragment_law_41, title_parts_law_41]
  rfl
theorem fixture_reader_58 : validSites o ⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩ = true := geometry_law_42
  change validSites o ⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n") && titlePartsLaw (nativeRead .md "- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n") = true
  rw [hg, native_read_42, fragment_law_42, title_parts_law_42]
  rfl
theorem fixture_reader_59 : validSites o ⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩ = true := geometry_law_43
  change validSites o ⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "title:: A\n- alias:: X\n  - kid\n- source\n") && titlePartsLaw (nativeRead .md "title:: A\n- alias:: X\n  - kid\n- source\n") = true
  rw [hg, native_read_43, fragment_law_43, title_parts_law_43]
  rfl
theorem fixture_reader_60 : validSites o ⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩ = true := geometry_law_44
  change validSites o ⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n") = true
  rw [hg, native_read_44, fragment_law_44, title_parts_law_44]
  rfl
theorem fixture_reader_61 : validSites o ⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩ && fragmentLaw .md (o.content ⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩) && titlePartsLaw (o.content ⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩) = true := by
  have hg : validSites o ⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩ = true := geometry_law_45
  change validSites o ⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- owner\n") && titlePartsLaw (nativeRead .md "alias:: X\n- owner\n") = true
  rw [hg, native_read_45, fragment_law_45, title_parts_law_45]
  rfl
theorem fixture_reader_62 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_63 : validSites o ⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩ && fragmentLaw .md (o.content ⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩) && titlePartsLaw (o.content ⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩) = true := by
  have hg : validSites o ⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩ = true := geometry_law_46
  change validSites o ⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩ && fragmentLaw .md (nativeRead .md "title:: X\n- p\n") && titlePartsLaw (nativeRead .md "title:: X\n- p\n") = true
  rw [hg, native_read_46, fragment_law_46, title_parts_law_46]
  rfl
theorem fixture_reader_64 : validSites o ⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩) && titlePartsLaw (o.content ⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩ = true := geometry_law_47
  change validSites o ⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩ && fragmentLaw .md (nativeRead .md "title:: X\n- q\n") && titlePartsLaw (nativeRead .md "title:: X\n- q\n") = true
  rw [hg, native_read_47, fragment_law_47, title_parts_law_47]
  rfl
theorem fixture_reader_65 : validSites o ⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩ && fragmentLaw .md (o.content ⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩) && titlePartsLaw (o.content ⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩) = true := by
  have hg : validSites o ⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩ = true := geometry_law_48
  change validSites o ⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩ && fragmentLaw .md (nativeRead .md "title:: A/x\n- p\n") && titlePartsLaw (nativeRead .md "title:: A/x\n- p\n") = true
  rw [hg, native_read_48, fragment_law_48, title_parts_law_48]
  rfl
theorem fixture_reader_66 : validSites o ⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩) && titlePartsLaw (o.content ⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩ = true := geometry_law_49
  change validSites o ⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩ && fragmentLaw .md (nativeRead .md "title:: A/x\n- q\n") && titlePartsLaw (nativeRead .md "title:: A/x\n- q\n") = true
  rw [hg, native_read_49, fragment_law_49, title_parts_law_49]
  rfl
theorem fixture_reader_67 : validSites o ⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩ && fragmentLaw .md (o.content ⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩) && titlePartsLaw (o.content ⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩) = true := by
  have hg : validSites o ⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩ = true := geometry_law_50
  change validSites o ⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩ && fragmentLaw .md (nativeRead .md "title:: C\n- occupied\n") && titlePartsLaw (nativeRead .md "title:: C\n- occupied\n") = true
  rw [hg, native_read_50, fragment_law_50, title_parts_law_50]
  rfl
theorem fixture_reader_68 : validSites o ⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩ = true := geometry_law_51
  change validSites o ⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- alias:: X\n  - kid\n- source\n") && titlePartsLaw (nativeRead .md "- alias:: X\n  - kid\n- source\n") = true
  rw [hg, native_read_51, fragment_law_51, title_parts_law_51]
  rfl
theorem fixture_reader_69 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_70 : validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ = true := geometry_law_33
  change validSites o ⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: Old/[[New]] [[X]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: Old/[[New]] [[X]]\n- child\n") = true
  rw [hg, native_read_33, fragment_law_33, title_parts_law_33]
  rfl
theorem fixture_reader_71 : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_72 : validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ = true := geometry_law_35
  change validSites o ⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: New/[[New]] [[X]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: New/[[New]] [[X]]\n- child\n") = true
  rw [hg, native_read_35, fragment_law_35, title_parts_law_35]
  rfl
theorem fixture_reader_73 : validSites o ⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ = true := geometry_law_36
  change validSites o ⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: Old/[[Old]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: Old/[[Old]]\n- child\n") = true
  rw [hg, native_read_36, fragment_law_36, title_parts_law_36]
  rfl
theorem fixture_reader_74 : validSites o ⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩ = true := geometry_law_52
  change validSites o ⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩ && fragmentLaw .md (nativeRead .md "title:: New/[[New]]\n- child\n") && titlePartsLaw (nativeRead .md "title:: New/[[New]]\n- child\n") = true
  rw [hg, native_read_52, fragment_law_52, title_parts_law_52]
  rfl
theorem fixture_reader_75 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_76 : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_77 : validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ = true := geometry_law_53
  change validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ && fragmentLaw .md (nativeRead .md "title:: [[X]]\n- notes\n") && titlePartsLaw (nativeRead .md "title:: [[X]]\n- notes\n") = true
  rw [hg, native_read_53, fragment_law_53, title_parts_law_53]
  rfl
theorem fixture_reader_78 : validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ = true := geometry_law_53
  change validSites o ⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩ && fragmentLaw .md (nativeRead .md "title:: [[X]]\n- notes\n") && titlePartsLaw (nativeRead .md "title:: [[X]]\n- notes\n") = true
  rw [hg, native_read_53, fragment_law_53, title_parts_law_53]
  rfl
theorem fixture_reader_79 : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ = true := geometry_law_2
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[B]]\n") && titlePartsLaw (nativeRead .md "- [[B]]\n") = true
  rw [hg, native_read_2, fragment_law_2, title_parts_law_2]
  rfl
theorem fixture_reader_80 : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ = true := geometry_law_2
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[B]]\n") && titlePartsLaw (nativeRead .md "- [[B]]\n") = true
  rw [hg, native_read_2, fragment_law_2, title_parts_law_2]
  rfl
theorem fixture_reader_81 : validSites o ⟨"pages/2026.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/2026.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/2026.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/2026.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/2026.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_82 : validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ && fragmentLaw .md (o.content ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩) && titlePartsLaw (o.content ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩) = true := by
  have hg : validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ = true := geometry_law_54
  change validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ && fragmentLaw .md (nativeRead .md "title:: 2026/10/26\n- day\n") && titlePartsLaw (nativeRead .md "title:: 2026/10/26\n- day\n") = true
  rw [hg, native_read_54, fragment_law_54, title_parts_law_54]
  rfl
theorem fixture_reader_83 : validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩ = true := geometry_law_55
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[2026/10/26]] [[2026/x]]\n") && titlePartsLaw (nativeRead .md "- [[2026/10/26]] [[2026/x]]\n") = true
  rw [hg, native_read_55, fragment_law_55, title_parts_law_55]
  rfl
theorem fixture_reader_84 : validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/B.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_85 : validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ && fragmentLaw .md (o.content ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩) && titlePartsLaw (o.content ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩) = true := by
  have hg : validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ = true := geometry_law_54
  change validSites o ⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ && fragmentLaw .md (nativeRead .md "title:: 2026/10/26\n- day\n") && titlePartsLaw (nativeRead .md "title:: 2026/10/26\n- day\n") = true
  rw [hg, native_read_54, fragment_law_54, title_parts_law_54]
  rfl
theorem fixture_reader_86 : validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩ = true := geometry_law_56
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[2026/10/26]] [[B/x]]\n") && titlePartsLaw (nativeRead .md "- [[2026/10/26]] [[B/x]]\n") = true
  rw [hg, native_read_56, fragment_law_56, title_parts_law_56]
  rfl
theorem fixture_reader_87 : validSites o ⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩ = true := geometry_law_57
  change validSites o ⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩ && fragmentLaw .md (nativeRead .md "join:: value\n- s\n") && titlePartsLaw (nativeRead .md "join:: value\n- s\n") = true
  rw [hg, native_read_57, fragment_law_57, title_parts_law_57]
  rfl
theorem fixture_reader_88 : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ = true := geometry_law_7
  change validSites o ⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩ && fragmentLaw .md (nativeRead .md "clash:: target\n- t\n") && titlePartsLaw (nativeRead .md "clash:: target\n- t\n") = true
  rw [hg, native_read_7, fragment_law_7, title_parts_law_7]
  rfl
theorem fixture_reader_89 : validSites o ⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩ = true := geometry_law_58
  change validSites o ⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩ && fragmentLaw .md (nativeRead .md "join:: value\nclash:: target\n- t\n- s\n") && titlePartsLaw (nativeRead .md "join:: value\nclash:: target\n- t\n- s\n") = true
  rw [hg, native_read_58, fragment_law_58, title_parts_law_58]
  rfl
theorem fixture_reader_90 : validSites o ⟨"pages/A.md",.pages,.md,"\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"\n- source\n"⟩ = true := geometry_law_59
  change validSites o ⟨"pages/A.md",.pages,.md,"\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "\n- source\n") && titlePartsLaw (nativeRead .md "\n- source\n") = true
  rw [hg, native_read_59, fragment_law_59, title_parts_law_59]
  rfl
theorem fixture_reader_91 : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ = true := geometry_law_5
  change validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n- source\n") && titlePartsLaw (nativeRead .md "- target\n- source\n") = true
  rw [hg, native_read_5, fragment_law_5, title_parts_law_5]
  rfl
theorem fixture_reader_92 : validSites o ⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩ = true := geometry_law_60
  change validSites o ⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- \n- source\n") && titlePartsLaw (nativeRead .md "- \n- source\n") = true
  rw [hg, native_read_60, fragment_law_60, title_parts_law_60]
  rfl
theorem fixture_reader_93 : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ = true := geometry_law_5
  change validSites o ⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n- source\n") && titlePartsLaw (nativeRead .md "- target\n- source\n") = true
  rw [hg, native_read_5, fragment_law_5, title_parts_law_5]
  rfl
theorem fixture_reader_94 : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_95 : validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ && fragmentLaw .org (o.content ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩) && titlePartsLaw (o.content ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ = true := geometry_law_13
  change validSites o ⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩ && fragmentLaw .org (nativeRead .org "* h :Old:\n") && titlePartsLaw (nativeRead .org "* h :Old:\n") = true
  rw [hg, native_read_13, fragment_law_13, title_parts_law_13]
  rfl
theorem fixture_reader_96 : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) && titlePartsLaw (o.content ⟨"pages/New.md",.pages,.md,"- notes\n"⟩) = true := by
  have hg : validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ = true := geometry_law_0
  change validSites o ⟨"pages/New.md",.pages,.md,"- notes\n"⟩ && fragmentLaw .md (nativeRead .md "- notes\n") && titlePartsLaw (nativeRead .md "- notes\n") = true
  rw [hg, native_read_0, fragment_law_0, title_parts_law_0]
  rfl
theorem fixture_reader_97 : validSites o ⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩ && fragmentLaw .org (o.content ⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩) && titlePartsLaw (o.content ⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩ = true := geometry_law_61
  change validSites o ⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩ && fragmentLaw .org (nativeRead .org "* h :New:\n") && titlePartsLaw (nativeRead .org "* h :New:\n") = true
  rw [hg, native_read_61, fragment_law_61, title_parts_law_61]
  rfl
theorem fixture_reader_98 : validSites o ⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩ && fragmentLaw .md (o.content ⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩) && titlePartsLaw (o.content ⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩) = true := by
  have hg : validSites o ⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩ = true := geometry_law_62
  change validSites o ⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩ && fragmentLaw .md (nativeRead .md "- [[A/x]]\n") && titlePartsLaw (nativeRead .md "- [[A/x]]\n") = true
  rw [hg, native_read_62, fragment_law_62, title_parts_law_62]
  rfl
theorem fixture_reader_99 : validSites o ⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩) && titlePartsLaw (o.content ⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩ = true := geometry_law_63
  change validSites o ⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩ && fragmentLaw .md (nativeRead .md "- bx\n") && titlePartsLaw (nativeRead .md "- bx\n") = true
  rw [hg, native_read_63, fragment_law_63, title_parts_law_63]
  rfl
theorem fixture_reader_100 : validSites o ⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩ && fragmentLaw .md (o.content ⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩) && titlePartsLaw (o.content ⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩) = true := by
  have hg : validSites o ⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩ = true := geometry_law_64
  change validSites o ⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: A/sub\n- owner\n") && titlePartsLaw (nativeRead .md "alias:: A/sub\n- owner\n") = true
  rw [hg, native_read_64, fragment_law_64, title_parts_law_64]
  rfl
theorem fixture_reader_101 : validSites o ⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩) && titlePartsLaw (o.content ⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩ = true := geometry_law_65
  change validSites o ⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩ && fragmentLaw .md (nativeRead .md "- bs\n") && titlePartsLaw (nativeRead .md "- bs\n") = true
  rw [hg, native_read_65, fragment_law_65, title_parts_law_65]
  rfl
theorem fixture_reader_102 : validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ = true := geometry_law_66
  change validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ && fragmentLaw .md (nativeRead .md "- child\n") && titlePartsLaw (nativeRead .md "- child\n") = true
  rw [hg, native_read_66, fragment_law_66, title_parts_law_66]
  rfl
theorem fixture_reader_103 : validSites o ⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩ = true := geometry_law_67
  change validSites o ⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- source\n") && titlePartsLaw (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- source\n") = true
  rw [hg, native_read_67, fragment_law_67, title_parts_law_67]
  rfl
theorem fixture_reader_104 : validSites o ⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩ = true := geometry_law_68
  change validSites o ⟨"pages/B.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- target\n") && titlePartsLaw (nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- target\n") = true
  rw [hg, native_read_68, fragment_law_68, title_parts_law_68]
  rfl
theorem fixture_reader_105 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ = true := geometry_law_23
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: X\n- source\n") && titlePartsLaw (nativeRead .md "alias:: X\n- source\n") = true
  rw [hg, native_read_23, fragment_law_23, title_parts_law_23]
  rfl
theorem fixture_reader_106 : validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩ = true := geometry_law_69
  change validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: Y\n- target\n") && titlePartsLaw (nativeRead .md "alias:: Y\n- target\n") = true
  rw [hg, native_read_69, fragment_law_69, title_parts_law_69]
  rfl
theorem fixture_reader_107 : validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩ = true := geometry_law_70
  change validSites o ⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: Y\nalias:: X\n- target\n- source\n") && titlePartsLaw (nativeRead .md "alias:: Y\nalias:: X\n- target\n- source\n") = true
  rw [hg, native_read_70, fragment_law_70, title_parts_law_70]
  rfl
theorem fixture_reader_108 : validSites o ⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩ = true := geometry_law_71
  change validSites o ⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "alias:: B/sub\n- source\n") && titlePartsLaw (nativeRead .md "alias:: B/sub\n- source\n") = true
  rw [hg, native_read_71, fragment_law_71, title_parts_law_71]
  rfl
theorem fixture_reader_109 : validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩) && titlePartsLaw (o.content ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ = true := geometry_law_66
  change validSites o ⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩ && fragmentLaw .md (nativeRead .md "- child\n") && titlePartsLaw (nativeRead .md "- child\n") = true
  rw [hg, native_read_66, fragment_law_66, title_parts_law_66]
  rfl
theorem fixture_reader_110 : validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ = true := geometry_law_72
  change validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "#+key: [[X]]\n* source\n") && titlePartsLaw (nativeRead .org "#+key: [[X]]\n* source\n") = true
  rw [hg, native_read_72, fragment_law_72, title_parts_law_72]
  rfl
theorem fixture_reader_111 : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ = true := geometry_law_73
  change validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ && fragmentLaw .org (nativeRead .org "#+key: target\n* target\n") && titlePartsLaw (nativeRead .org "#+key: target\n* target\n") = true
  rw [hg, native_read_73, fragment_law_73, title_parts_law_73]
  rfl
theorem fixture_reader_112 : validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ = true := geometry_law_74
  change validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "j:: J\nk:: K\n- source\n") && titlePartsLaw (nativeRead .md "j:: J\nk:: K\n- source\n") = true
  rw [hg, native_read_74, fragment_law_74, title_parts_law_74]
  rfl
theorem fixture_reader_113 : validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ = true := geometry_law_75
  change validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "x:: X\ny:: Y\n- target\n") && titlePartsLaw (nativeRead .md "x:: X\ny:: Y\n- target\n") = true
  rw [hg, native_read_75, fragment_law_75, title_parts_law_75]
  rfl
theorem fixture_reader_114 : validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩ = true := geometry_law_76
  change validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n") && titlePartsLaw (nativeRead .md "k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n") = true
  rw [hg, native_read_76, fragment_law_76, title_parts_law_76]
  rfl
theorem fixture_reader_115 : validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ = true := geometry_law_74
  change validSites o ⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "j:: J\nk:: K\n- source\n") && titlePartsLaw (nativeRead .md "j:: J\nk:: K\n- source\n") = true
  rw [hg, native_read_74, fragment_law_74, title_parts_law_74]
  rfl
theorem fixture_reader_116 : validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ = true := geometry_law_75
  change validSites o ⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "x:: X\ny:: Y\n- target\n") && titlePartsLaw (nativeRead .md "x:: X\ny:: Y\n- target\n") = true
  rw [hg, native_read_75, fragment_law_75, title_parts_law_75]
  rfl
theorem fixture_reader_117 : validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩ = true := geometry_law_77
  change validSites o ⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "k:: K\nx:: X\ny:: Y\n- target\n- source\n") && titlePartsLaw (nativeRead .md "k:: K\nx:: X\ny:: Y\n- target\n- source\n") = true
  rw [hg, native_read_77, fragment_law_77, title_parts_law_77]
  rfl
theorem fixture_reader_118 : validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ = true := geometry_law_72
  change validSites o ⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "#+key: [[X]]\n* source\n") && titlePartsLaw (nativeRead .org "#+key: [[X]]\n* source\n") = true
  rw [hg, native_read_72, fragment_law_72, title_parts_law_72]
  rfl
theorem fixture_reader_119 : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ = true := geometry_law_73
  change validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩ && fragmentLaw .org (nativeRead .org "#+key: target\n* target\n") && titlePartsLaw (nativeRead .org "#+key: target\n* target\n") = true
  rw [hg, native_read_73, fragment_law_73, title_parts_law_73]
  rfl
theorem fixture_reader_120 : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩ = true := geometry_law_78
  change validSites o ⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n") && titlePartsLaw (nativeRead .org "#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n") = true
  rw [hg, native_read_78, fragment_law_78, title_parts_law_78]
  rfl
theorem fixture_reader_121 : validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ = true := geometry_law_79
  change validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "key:: [[X]]\n- source\n") && titlePartsLaw (nativeRead .md "key:: [[X]]\n- source\n") = true
  rw [hg, native_read_79, fragment_law_79, title_parts_law_79]
  rfl
theorem fixture_reader_122 : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ = true := geometry_law_80
  change validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "key:: target\n- target\n") && titlePartsLaw (nativeRead .md "key:: target\n- target\n") = true
  rw [hg, native_read_80, fragment_law_80, title_parts_law_80]
  rfl
theorem fixture_reader_123 : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩ = true := geometry_law_81
  change validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "key:: target\n- target\n- \n  key:: [[X]]\n- source\n") && titlePartsLaw (nativeRead .md "key:: target\n- target\n- \n  key:: [[X]]\n- source\n") = true
  rw [hg, native_read_81, fragment_law_81, title_parts_law_81]
  rfl
theorem fixture_reader_124 : validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ = true := geometry_law_79
  change validSites o ⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "key:: [[X]]\n- source\n") && titlePartsLaw (nativeRead .md "key:: [[X]]\n- source\n") = true
  rw [hg, native_read_79, fragment_law_79, title_parts_law_79]
  rfl
theorem fixture_reader_125 : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ = true := geometry_law_80
  change validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "key:: target\n- target\n") && titlePartsLaw (nativeRead .md "key:: target\n- target\n") = true
  rw [hg, native_read_80, fragment_law_80, title_parts_law_80]
  rfl
theorem fixture_reader_126 : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩ = true := geometry_law_82
  change validSites o ⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "key:: target\n- target\n- \n  key:: X\n- source\n") && titlePartsLaw (nativeRead .md "key:: target\n- target\n- \n  key:: X\n- source\n") = true
  rw [hg, native_read_82, fragment_law_82, title_parts_law_82]
  rfl
theorem fixture_reader_127 : validSites o ⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (o.content ⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩ = true := geometry_law_83
  change validSites o ⟨"pages/B.org",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩ && fragmentLaw .org (nativeRead .org "* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n") && titlePartsLaw (nativeRead .org "* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n") = true
  rw [hg, native_read_83, fragment_law_83, title_parts_law_83]
  rfl
theorem fixture_reader_128 : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩ = true := geometry_law_164
  change validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: star\npublic:: true\n- source\n") && titlePartsLaw (nativeRead .md "icon:: star\npublic:: true\n- source\n") = true
  rw [hg, native_read_164, fragment_law_164, title_parts_law_164]
  rfl
theorem fixture_reader_129 : validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ = true := geometry_law_4
  change validSites o ⟨"pages/B.md",.pages,.md,"- target\n"⟩ && fragmentLaw .md (nativeRead .md "- target\n") && titlePartsLaw (nativeRead .md "- target\n") = true
  rw [hg, native_read_4, fragment_law_4, title_parts_law_4]
  rfl
theorem fixture_reader_130 : validSites o ⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩ = true := geometry_law_165
  change validSites o ⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: star\npublic:: true\n- target\n- source\n") && titlePartsLaw (nativeRead .md "icon:: star\npublic:: true\n- target\n- source\n") = true
  rw [hg, native_read_165, fragment_law_165, title_parts_law_165]
  rfl
theorem fixture_reader_131 : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ = true := geometry_law_166
  change validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: star\n- source\n") && titlePartsLaw (nativeRead .md "icon:: star\n- source\n") = true
  rw [hg, native_read_166, fragment_law_166, title_parts_law_166]
  rfl
theorem fixture_reader_132 : validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩ = true := geometry_law_167
  change validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: moon\n- target\n") && titlePartsLaw (nativeRead .md "icon:: moon\n- target\n") = true
  rw [hg, native_read_167, fragment_law_167, title_parts_law_167]
  rfl
theorem fixture_reader_133 : validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩ = true := geometry_law_168
  change validSites o ⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: moon\n- target\n- \n  icon:: star\n- source\n") && titlePartsLaw (nativeRead .md "icon:: moon\n- target\n- \n  icon:: star\n- source\n") = true
  rw [hg, native_read_168, fragment_law_168, title_parts_law_168]
  rfl
theorem fixture_reader_134 : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ = true := geometry_law_166
  change validSites o ⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "icon:: star\n- source\n") && titlePartsLaw (nativeRead .md "icon:: star\n- source\n") = true
  rw [hg, native_read_166, fragment_law_166, title_parts_law_166]
  rfl
theorem fixture_reader_135 : validSites o ⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩ = true := geometry_law_169
  change validSites o ⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "public:: true\n- target\n") && titlePartsLaw (nativeRead .md "public:: true\n- target\n") = true
  rw [hg, native_read_169, fragment_law_169, title_parts_law_169]
  rfl
theorem fixture_reader_136 : validSites o ⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩ = true := geometry_law_170
  change validSites o ⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "public:: true\nicon:: star\n- target\n- source\n") && titlePartsLaw (nativeRead .md "public:: true\nicon:: star\n- target\n- source\n") = true
  rw [hg, native_read_170, fragment_law_170, title_parts_law_170]
  rfl
theorem fixture_reader_137 : validSites o ⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩ = true := geometry_law_171
  change validSites o ⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "Old:: source\n- source\n") && titlePartsLaw (nativeRead .md "Old:: source\n- source\n") = true
  rw [hg, native_read_171, fragment_law_171, title_parts_law_171]
  rfl
theorem fixture_reader_138 : validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩) && titlePartsLaw (o.content ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩ = true := geometry_law_172
  change validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩ && fragmentLaw .md (nativeRead .md "two-words:: target\n- target\n") && titlePartsLaw (nativeRead .md "two-words:: target\n- target\n") = true
  rw [hg, native_read_172, fragment_law_172, title_parts_law_172]
  rfl
theorem fixture_reader_139 : validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩ && fragmentLaw .md (o.content ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩) && titlePartsLaw (o.content ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩) = true := by
  have hg : validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩ = true := geometry_law_173
  change validSites o ⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩ && fragmentLaw .md (nativeRead .md "two-words:: target\n- target\n- \n  two-words:: source\n- source\n") && titlePartsLaw (nativeRead .md "two-words:: target\n- target\n- \n  two-words:: source\n- source\n") = true
  rw [hg, native_read_173, fragment_law_173, title_parts_law_173]
  rfl
theorem fixtures_reader_laws : fixtureFiles.all (fun f => validSites o f && fragmentLaw f.format (o.content f) && titlePartsLaw (o.content f)) = true := by
  apply List.all_eq_true.mpr
  intro f hf
  simp only [fixtureFiles,List.mem_cons,List.not_mem_nil,or_false] at hf
  rcases hf with h0 | h1 | h2 | h3 | h4 | h5 | h6 | h7 | h8 | h9 | h10 | h11 | h12 | h13 | h14 | h15 | h16 | h17 | h18 | h19 | h20 | h21 | h22 | h23 | h24 | h25 | h26 | h27 | h28 | h29 | h30 | h31 | h32 | h33 | h34 | h35 | h36 | h37 | h38 | h39 | h40 | h41 | h42 | h43 | h44 | h45 | h46 | h47 | h48 | h49 | h50 | h51 | h52 | h53 | h54 | h55 | h56 | h57 | h58 | h59 | h60 | h61 | h62 | h63 | h64 | h65 | h66 | h67 | h68 | h69 | h70 | h71 | h72 | h73 | h74 | h75 | h76 | h77 | h78 | h79 | h80 | h81 | h82 | h83 | h84 | h85 | h86 | h87 | h88 | h89 | h90 | h91 | h92 | h93 | h94 | h95 | h96 | h97 | h98 | h99 | h100 | h101 | h102 | h103 | h104 | h105 | h106 | h107 | h108 | h109 | h110 | h111 | h112 | h113 | h114 | h115 | h116 | h117 | h118 | h119 | h120 | h121 | h122 | h123 | h124 | h125 | h126 | h127 | h128 | h129 | h130 | h131 | h132 | h133 | h134 | h135 | h136 | h137 | h138 | h139
  · cases h0; exact fixture_reader_0
  · cases h1; exact fixture_reader_1
  · cases h2; exact fixture_reader_2
  · cases h3; exact fixture_reader_3
  · cases h4; exact fixture_reader_4
  · cases h5; exact fixture_reader_5
  · cases h6; exact fixture_reader_6
  · cases h7; exact fixture_reader_7
  · cases h8; exact fixture_reader_8
  · cases h9; exact fixture_reader_9
  · cases h10; exact fixture_reader_10
  · cases h11; exact fixture_reader_11
  · cases h12; exact fixture_reader_12
  · cases h13; exact fixture_reader_13
  · cases h14; exact fixture_reader_14
  · cases h15; exact fixture_reader_15
  · cases h16; exact fixture_reader_16
  · cases h17; exact fixture_reader_17
  · cases h18; exact fixture_reader_18
  · cases h19; exact fixture_reader_19
  · cases h20; exact fixture_reader_20
  · cases h21; exact fixture_reader_21
  · cases h22; exact fixture_reader_22
  · cases h23; exact fixture_reader_23
  · cases h24; exact fixture_reader_24
  · cases h25; exact fixture_reader_25
  · cases h26; exact fixture_reader_26
  · cases h27; exact fixture_reader_27
  · cases h28; exact fixture_reader_28
  · cases h29; exact fixture_reader_29
  · cases h30; exact fixture_reader_30
  · cases h31; exact fixture_reader_31
  · cases h32; exact fixture_reader_32
  · cases h33; exact fixture_reader_33
  · cases h34; exact fixture_reader_34
  · cases h35; exact fixture_reader_35
  · cases h36; exact fixture_reader_36
  · cases h37; exact fixture_reader_37
  · cases h38; exact fixture_reader_38
  · cases h39; exact fixture_reader_39
  · cases h40; exact fixture_reader_40
  · cases h41; exact fixture_reader_41
  · cases h42; exact fixture_reader_42
  · cases h43; exact fixture_reader_43
  · cases h44; exact fixture_reader_44
  · cases h45; exact fixture_reader_45
  · cases h46; exact fixture_reader_46
  · cases h47; exact fixture_reader_47
  · cases h48; exact fixture_reader_48
  · cases h49; exact fixture_reader_49
  · cases h50; exact fixture_reader_50
  · cases h51; exact fixture_reader_51
  · cases h52; exact fixture_reader_52
  · cases h53; exact fixture_reader_53
  · cases h54; exact fixture_reader_54
  · cases h55; exact fixture_reader_55
  · cases h56; exact fixture_reader_56
  · cases h57; exact fixture_reader_57
  · cases h58; exact fixture_reader_58
  · cases h59; exact fixture_reader_59
  · cases h60; exact fixture_reader_60
  · cases h61; exact fixture_reader_61
  · cases h62; exact fixture_reader_62
  · cases h63; exact fixture_reader_63
  · cases h64; exact fixture_reader_64
  · cases h65; exact fixture_reader_65
  · cases h66; exact fixture_reader_66
  · cases h67; exact fixture_reader_67
  · cases h68; exact fixture_reader_68
  · cases h69; exact fixture_reader_69
  · cases h70; exact fixture_reader_70
  · cases h71; exact fixture_reader_71
  · cases h72; exact fixture_reader_72
  · cases h73; exact fixture_reader_73
  · cases h74; exact fixture_reader_74
  · cases h75; exact fixture_reader_75
  · cases h76; exact fixture_reader_76
  · cases h77; exact fixture_reader_77
  · cases h78; exact fixture_reader_78
  · cases h79; exact fixture_reader_79
  · cases h80; exact fixture_reader_80
  · cases h81; exact fixture_reader_81
  · cases h82; exact fixture_reader_82
  · cases h83; exact fixture_reader_83
  · cases h84; exact fixture_reader_84
  · cases h85; exact fixture_reader_85
  · cases h86; exact fixture_reader_86
  · cases h87; exact fixture_reader_87
  · cases h88; exact fixture_reader_88
  · cases h89; exact fixture_reader_89
  · cases h90; exact fixture_reader_90
  · cases h91; exact fixture_reader_91
  · cases h92; exact fixture_reader_92
  · cases h93; exact fixture_reader_93
  · cases h94; exact fixture_reader_94
  · cases h95; exact fixture_reader_95
  · cases h96; exact fixture_reader_96
  · cases h97; exact fixture_reader_97
  · cases h98; exact fixture_reader_98
  · cases h99; exact fixture_reader_99
  · cases h100; exact fixture_reader_100
  · cases h101; exact fixture_reader_101
  · cases h102; exact fixture_reader_102
  · cases h103; exact fixture_reader_103
  · cases h104; exact fixture_reader_104
  · cases h105; exact fixture_reader_105
  · cases h106; exact fixture_reader_106
  · cases h107; exact fixture_reader_107
  · cases h108; exact fixture_reader_108
  · cases h109; exact fixture_reader_109
  · cases h110; exact fixture_reader_110
  · cases h111; exact fixture_reader_111
  · cases h112; exact fixture_reader_112
  · cases h113; exact fixture_reader_113
  · cases h114; exact fixture_reader_114
  · cases h115; exact fixture_reader_115
  · cases h116; exact fixture_reader_116
  · cases h117; exact fixture_reader_117
  · cases h118; exact fixture_reader_118
  · cases h119; exact fixture_reader_119
  · cases h120; exact fixture_reader_120
  · cases h121; exact fixture_reader_121
  · cases h122; exact fixture_reader_122
  · cases h123; exact fixture_reader_123
  · cases h124; exact fixture_reader_124
  · cases h125; exact fixture_reader_125
  · cases h126; exact fixture_reader_126
  · cases h127; exact fixture_reader_127
  · cases h128; exact fixture_reader_128
  · cases h129; exact fixture_reader_129
  · cases h130; exact fixture_reader_130
  · cases h131; exact fixture_reader_131
  · cases h132; exact fixture_reader_132
  · cases h133; exact fixture_reader_133
  · cases h134; exact fixture_reader_134
  · cases h135; exact fixture_reader_135
  · cases h136; exact fixture_reader_136
  · cases h137; exact fixture_reader_137
  · cases h138; exact fixture_reader_138
  · cases h139; exact fixture_reader_139

theorem mixed_exceptions_locations : exceptionReports p dates o mixed_exceptions_before "Old" ⟨"Two Words",some ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,none⟩ = [⟨"pages/Conflict.md",30,37,.skippedConflict,.link⟩,⟨"pages/Org.org",4,9,.unspellable,.tag⟩,⟨"pages/OrgConflict.org",32,37,.skippedConflict,.tag⟩] := by decide +kernel
theorem key_spelling_locations : exceptionReports p dates o key_spelling_before "Old" ⟨"Two Words",some ⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,none⟩ = [⟨"pages/R.md",2,5,.normalizedKey,.propertyKey⟩,⟨"pages/R.md",16,19,.normalizedKey,.propertyKey⟩] := by decide +kernel


end CutExamples
end CM
