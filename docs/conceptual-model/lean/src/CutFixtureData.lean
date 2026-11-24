import CutRefactorProofs
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def asciiLower (s : String) : String := String.ofList (s.toList.map (fun c =>
  if 65 ≤ c.toNat ∧ c.toNat ≤ 90 then Char.ofNat (c.toNat + 32) else c))
def dates : Date13.Dates String := ⟨fun s => if s = "2026-11-02" || s = "2026/10/26" then some s else none,fun _ => none⟩
def spell (fmt : Format) (s : Site) (n : String) : Option String :=
  if s.context = .splitLine then none
  else if s.context = .orgHeadlineTag && (n.toList.contains ' ' || n.toList.contains ':') then none
  else if (s.context = .queryArg ∨ s.context = .macroArgument) && (n.toList.contains ' ' || n.toList.contains '}') then none
  else if s.kind = .propertyKey && n.toList.contains ':' then none
  else some (match s.kind with
    | .link => "[[" ++ n ++ "]]"
    | .tag => if fmt = .org then ":" ++ n ++ ":" else if n.toList.contains ' ' then "#[[" ++ n ++ "]]" else "#" ++ n
    | _ => n)
def nativeContent_0 : Content := ⟨[],[⟨0,[.text "notes"],false⟩]⟩
def nativeContent_1 : Content := ⟨[],[⟨0,[.site ⟨2,7,.link,.ordinary,"A","[[A]]"⟩],false⟩]⟩
def nativeContent_2 : Content := ⟨[],[⟨0,[.site ⟨2,7,.link,.ordinary,"B","[[B]]"⟩],false⟩]⟩
def nativeContent_3 : Content := ⟨[],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_4 : Content := ⟨[],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_5 : Content := ⟨[],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_6 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,5,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "source"],(some "clash")⟩,⟨.property,.markdown,[.site ⟨15,19,.propertyKey,.ordinary,"join","join"⟩,.text ":: ",.text "value"],(some "join")⟩],[⟨0,[.text "s"],false⟩]⟩
def nativeContent_7 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,5,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "target"],(some "clash")⟩],[⟨0,[.text "t"],false⟩]⟩
def nativeContent_8 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,5,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "target"],(some "clash")⟩,⟨.property,.markdown,[.site ⟨15,19,.propertyKey,.ordinary,"join","join"⟩,.text ":: ",.text "value"],(some "join")⟩],[⟨0,[.text "t"],false⟩,⟨0,[.text "\n",.site ⟨37,42,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: source"],false⟩,⟨0,[.text "s"],false⟩]⟩
def nativeContent_9 : Content := ⟨[⟨.title,.directive,[.text "#+title: ",.title ⟨9,10,.title,.ordinary,"A","A"⟩ [.inl "A"]],(some "title")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_10 : Content := ⟨[⟨.title,.directive,[.text "#+title: ",.title ⟨9,10,.title,.ordinary,"B","B"⟩ [.inl "B"]],(some "title")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_11 : Content := ⟨[⟨.title,.directive,[.text "#+title: ",.title ⟨9,10,.title,.ordinary,"B","B"⟩ [.inl "B"]],(some "title")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "#+BEGIN_QUOTE",.text "\n",.text "#+title: B",.text "\n",.text "#+END_QUOTE"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_12 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩,⟨0,[.site ⟨30,37,.link,.ordinary,"Old","[[Old]]"⟩],false⟩]⟩
def nativeContent_13 : Content := ⟨[],[⟨0,[.text "h ",.site ⟨4,9,.tag,.orgHeadlineTag,"Old",":Old:"⟩],false⟩]⟩
def nativeContent_14 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩,⟨0,[.text "h ",.site ⟨32,37,.tag,.orgHeadlineTag,"Old",":Old:"⟩],false⟩]⟩
def nativeContent_15 : Content := ⟨[],[⟨0,[.site ⟨2,6,.tag,.mdTag,"Old","#Old"⟩],false⟩]⟩
def nativeContent_16 : Content := ⟨[],[⟨0,[.site ⟨2,16,.tag,.mdTag,"Two Words","#[[Two Words]]"⟩],false⟩]⟩
def nativeContent_17 : Content := ⟨[],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_18 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,10,.aliasName,.ordinary,"C#","C#"⟩],(some "alias")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_19 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,11,.aliasName,.ordinary,"C#","C\\#"⟩],(some "alias")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_20 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,11,.aliasName,.ordinary,"C#","C\\#"⟩],(some "alias")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_21 : Content := ⟨[⟨.property,.markdown,[.site ⟨2,5,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "value"],(some "Old")⟩,⟨.property,.markdown,[.site ⟨16,19,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "another"],(some "Old")⟩],[]⟩
def nativeContent_22 : Content := ⟨[⟨.property,.markdown,[.site ⟨2,11,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "value"],(some "two-words")⟩,⟨.property,.markdown,[.site ⟨22,31,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "another"],(some "two-words")⟩],[]⟩
def nativeContent_23 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_24 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_25 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_26 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "title:: X"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_27 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "one"],(some "x")⟩,⟨.property,.markdown,[.site ⟨8,9,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "two"],(some "x")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_28 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "one"],(some "x")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.site ⟨22,23,.propertyKey,.ordinary,"x","x"⟩,.text ":: two"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_29 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"A","A"⟩],(some "alias")⟩],[⟨0,[.text "owner"],false⟩]⟩
def nativeContent_30 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"B","B"⟩],(some "alias")⟩],[⟨0,[.text "owner"],false⟩]⟩
def nativeContent_31 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,18,.title,.ordinary,"2026-11-02","2026-11-02"⟩ [.inl "2026-11-02"]],(some "title")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_32 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,18,.title,.ordinary,"2026-11-02","2026-11-02"⟩ [.inl "2026-11-02"]],(some "title")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_33 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,25,.title,.ordinary,"Old/[[New]] [[X]]","Old/[[New]] [[X]]"⟩ [.inl "Old/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩,.inl " ",.inr ⟨20,25,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_34 : Content := ⟨[],[⟨0,[.text "target"],false⟩,⟨0,[.text "notes"],false⟩]⟩
def nativeContent_35 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,25,.title,.ordinary,"New/[[New]] [[X]]","New/[[New]] [[X]]"⟩ [.inl "New/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩,.inl " ",.inr ⟨20,25,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_36 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"Old/[[Old]]","Old/[[Old]]"⟩ [.inl "Old/",.inr ⟨12,19,.link,.ordinary,"Old","[[Old]]"⟩]],(some "title")⟩],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_37 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"old/[[old]]","old/[[old]]"⟩ [.inl "old/",.inr ⟨12,19,.link,.ordinary,"old","[[old]]"⟩]],(some "title")⟩],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_38 : Content := ⟨[⟨.other,.markdown,[.text "alias::"],(some "alias")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_39 : Content := ⟨[⟨.other,.markdown,[.text "alias::"],(some "alias")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_40 : Content := ⟨[],[⟨0,[.text "source prose\n",.text "\n"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_41 : Content := ⟨[],[⟨0,[.text "target prose\n",.text "\n"],false⟩,⟨0,[.text "target"],false⟩]⟩
def nativeContent_42 : Content := ⟨[],[⟨0,[.text "target prose",.text "\n",.text "\n"],false⟩,⟨0,[.text "target"],false⟩,⟨0,[.text "source prose",.text "\n",.text "\n"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_43 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"A","A"⟩ [.inl "A"]],(some "title")⟩,⟨.aliases,.markdown,[.text "alias:: ",.site ⟨20,21,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨1,[.text "kid"],true⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_44 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "title:: B"],false⟩,⟨1,[.text "kid"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_45 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨0,[.text "owner"],false⟩]⟩
def nativeContent_46 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[⟨0,[.text "p"],false⟩]⟩
def nativeContent_47 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[⟨0,[.text "q"],false⟩]⟩
def nativeContent_48 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,11,.title,.ordinary,"A/x","A/x"⟩ [.inl "A/x"]],(some "title")⟩],[⟨0,[.text "p"],false⟩]⟩
def nativeContent_49 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,11,.title,.ordinary,"A/x","A/x"⟩ [.inl "A/x"]],(some "title")⟩],[⟨0,[.text "q"],false⟩]⟩
def nativeContent_50 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"C","C"⟩ [.inl "C"]],(some "title")⟩],[⟨0,[.text "occupied"],false⟩]⟩
def nativeContent_51 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨10,11,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨1,[.text "kid"],true⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_52 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"New/[[New]]","New/[[New]]"⟩ [.inl "New/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩]],(some "title")⟩],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_53 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,13,.title,.ordinary,"[[X]]","[[X]]"⟩ [.inr ⟨8,13,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[⟨0,[.text "notes"],false⟩]⟩
def nativeContent_54 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,18,.title,.ordinary,"2026/10/26","2026/10/26"⟩ [.inl "2026/10/26"]],(some "title")⟩],[⟨0,[.text "day"],false⟩]⟩
def nativeContent_55 : Content := ⟨[],[⟨0,[.site ⟨2,16,.link,.ordinary,"2026/10/26","[[2026/10/26]]"⟩,.text " ",.site ⟨17,27,.link,.ordinary,"2026/x","[[2026/x]]"⟩],false⟩]⟩
def nativeContent_56 : Content := ⟨[],[⟨0,[.site ⟨2,16,.link,.ordinary,"2026/10/26","[[2026/10/26]]"⟩,.text " ",.site ⟨17,24,.link,.ordinary,"B/x","[[B/x]]"⟩],false⟩]⟩
def nativeContent_57 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,4,.propertyKey,.ordinary,"join","join"⟩,.text ":: ",.text "value"],(some "join")⟩],[⟨0,[.text "s"],false⟩]⟩
def nativeContent_58 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,4,.propertyKey,.ordinary,"join","join"⟩,.text ":: ",.text "value"],(some "join")⟩,⟨.property,.markdown,[.site ⟨13,18,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "target"],(some "clash")⟩],[⟨0,[.text "t"],false⟩,⟨0,[.text "s"],false⟩]⟩
def nativeContent_59 : Content := ⟨[],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_60 : Content := ⟨[⟨.other,.other,[],none⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_61 : Content := ⟨[],[⟨0,[.text "h ",.site ⟨4,9,.tag,.orgHeadlineTag,"New",":New:"⟩],false⟩]⟩
def nativeContent_62 : Content := ⟨[],[⟨0,[.site ⟨2,9,.link,.ordinary,"A/x","[[A/x]]"⟩],false⟩]⟩
def nativeContent_63 : Content := ⟨[],[⟨0,[.text "bx"],false⟩]⟩
def nativeContent_64 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,13,.aliasName,.ordinary,"A/sub","A/sub"⟩],(some "alias")⟩],[⟨0,[.text "owner"],false⟩]⟩
def nativeContent_65 : Content := ⟨[],[⟨0,[.text "bs"],false⟩]⟩
def nativeContent_66 : Content := ⟨[],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_67 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_68 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩,⟨0,[.text "target"],false⟩]⟩
def nativeContent_69 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"Y","Y"⟩],(some "alias")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_70 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"Y","Y"⟩],(some "alias")⟩,⟨.aliases,.markdown,[.text "alias:: ",.site ⟨18,19,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_71 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,13,.aliasName,.ordinary,"B/sub","B/sub"⟩],(some "alias")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_72 : Content := ⟨[⟨.property,.directive,[.text "#+",.site ⟨2,5,.propertyKey,.ordinary,"key","key"⟩,.text ": ",.site ⟨7,12,.link,.ordinary,"X","[[X]]"⟩],(some "key")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_73 : Content := ⟨[⟨.property,.directive,[.text "#+",.site ⟨2,5,.propertyKey,.ordinary,"key","key"⟩,.text ": ",.text "target"],(some "key")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_74 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"j","j"⟩,.text ":: ",.text "J"],(some "j")⟩,⟨.property,.markdown,[.site ⟨6,7,.propertyKey,.ordinary,"k","k"⟩,.text ":: ",.text "K"],(some "k")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_75 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "X"],(some "x")⟩,⟨.property,.markdown,[.site ⟨6,7,.propertyKey,.ordinary,"y","y"⟩,.text ":: ",.text "Y"],(some "y")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_76 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"k","k"⟩,.text ":: ",.text "K"],(some "k")⟩,⟨.property,.markdown,[.site ⟨6,7,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "X"],(some "x")⟩,⟨.property,.markdown,[.site ⟨12,13,.propertyKey,.ordinary,"j","j"⟩,.text ":: ",.text "J"],(some "j")⟩,⟨.property,.markdown,[.site ⟨18,19,.propertyKey,.ordinary,"y","y"⟩,.text ":: ",.text "Y"],(some "y")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_77 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"k","k"⟩,.text ":: ",.text "K"],(some "k")⟩,⟨.property,.markdown,[.site ⟨6,7,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "X"],(some "x")⟩,⟨.property,.markdown,[.site ⟨12,13,.propertyKey,.ordinary,"y","y"⟩,.text ":: ",.text "Y"],(some "y")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_78 : Content := ⟨[⟨.property,.directive,[.text "#+",.site ⟨2,5,.propertyKey,.ordinary,"key","key"⟩,.text ": ",.text "target"],(some "key")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "#+BEGIN_QUOTE",.text "\n",.text "#+key: X",.text "\n",.text "#+END_QUOTE"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_79 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.site ⟨6,11,.link,.ordinary,"X","[[X]]"⟩],(some "key")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_80 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.text "target"],(some "key")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_81 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.text "target"],(some "key")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.site ⟨27,30,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.site ⟨33,38,.link,.ordinary,"X","[[X]]"⟩],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_82 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.text "target"],(some "key")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.site ⟨27,30,.propertyKey,.ordinary,"key","key"⟩,.text ":: X"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_83 : Content := ⟨[],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "#+BEGIN_QUOTE",.text "\n",.text "#+key: [[X]]",.text "\n",.text "#+END_QUOTE"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_84 : Content := ⟨[],[⟨0,[.text "notes"],false⟩]⟩
def nativeContent_85 : Content := ⟨[],[⟨0,[.site ⟨0,5,.link,.ordinary,"A","[[A]]"⟩],false⟩]⟩
def nativeContent_86 : Content := ⟨[],[⟨0,[.site ⟨0,5,.link,.ordinary,"B","[[B]]"⟩],false⟩]⟩
def nativeContent_87 : Content := ⟨[],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_88 : Content := ⟨[],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_89 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,5,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "source"],(some "clash")⟩],[]⟩
def nativeContent_90 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,4,.propertyKey,.ordinary,"join","join"⟩,.text ":: ",.text "value"],(some "join")⟩],[]⟩
def nativeContent_91 : Content := ⟨[],[⟨0,[.text "s"],false⟩]⟩
def nativeContent_92 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,5,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: ",.text "target"],(some "clash")⟩],[]⟩
def nativeContent_93 : Content := ⟨[],[⟨0,[.text "t"],false⟩]⟩
def nativeContent_94 : Content := ⟨[],[⟨0,[.text "\n",.site ⟨1,6,.propertyKey,.ordinary,"clash","clash"⟩,.text ":: source"],false⟩]⟩
def nativeContent_95 : Content := ⟨[⟨.title,.directive,[.text "#+title: ",.title ⟨9,10,.title,.ordinary,"A","A"⟩ [.inl "A"]],(some "title")⟩],[]⟩
def nativeContent_96 : Content := ⟨[],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_97 : Content := ⟨[⟨.title,.directive,[.text "#+title: ",.title ⟨9,10,.title,.ordinary,"B","B"⟩ [.inl "B"]],(some "title")⟩],[]⟩
def nativeContent_98 : Content := ⟨[],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_99 : Content := ⟨[],[⟨0,[.text "\n",.text "#+BEGIN_QUOTE\n",.text "#+title: B\n",.text "#+END_QUOTE"],false⟩]⟩
def nativeContent_100 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩]⟩
def nativeContent_101 : Content := ⟨[],[⟨0,[.site ⟨0,7,.link,.ordinary,"Old","[[Old]]"⟩],false⟩]⟩
def nativeContent_102 : Content := ⟨[],[⟨0,[.text "h ",.site ⟨2,7,.tag,.orgHeadlineTag,"Old",":Old:"⟩],false⟩]⟩
def nativeContent_103 : Content := ⟨[],[⟨0,[.text "<<<<<<< a\n",.text "=======\n",.text ">>>>>>> b\n"],false⟩]⟩
def nativeContent_104 : Content := ⟨[],[⟨0,[.site ⟨0,4,.tag,.mdTag,"Old","#Old"⟩],false⟩]⟩
def nativeContent_105 : Content := ⟨[],[⟨0,[.site ⟨0,14,.tag,.mdTag,"Two Words","#[[Two Words]]"⟩],false⟩]⟩
def nativeContent_106 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,10,.aliasName,.ordinary,"C#","C#"⟩],(some "alias")⟩],[]⟩
def nativeContent_107 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,11,.aliasName,.ordinary,"C#","C\\#"⟩],(some "alias")⟩],[]⟩
def nativeContent_108 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "value"],(some "Old")⟩],[]⟩
def nativeContent_109 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "another"],(some "Old")⟩],[]⟩
def nativeContent_110 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,9,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "value"],(some "two-words")⟩],[]⟩
def nativeContent_111 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,9,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "another"],(some "two-words")⟩],[]⟩
def nativeContent_112 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"X","X"⟩],(some "alias")⟩],[]⟩
def nativeContent_113 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"X","X"⟩ [.inl "X"]],(some "title")⟩],[]⟩
def nativeContent_114 : Content := ⟨[],[⟨0,[.text "\n",.text "title:: X"],false⟩]⟩
def nativeContent_115 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "one"],(some "x")⟩],[]⟩
def nativeContent_116 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "two"],(some "x")⟩],[]⟩
def nativeContent_117 : Content := ⟨[],[⟨0,[.text "\n",.site ⟨1,2,.propertyKey,.ordinary,"x","x"⟩,.text ":: two"],false⟩]⟩
def nativeContent_118 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"A","A"⟩],(some "alias")⟩],[]⟩
def nativeContent_119 : Content := ⟨[],[⟨0,[.text "owner"],false⟩]⟩
def nativeContent_120 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"B","B"⟩],(some "alias")⟩],[]⟩
def nativeContent_121 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,18,.title,.ordinary,"2026-11-02","2026-11-02"⟩ [.inl "2026-11-02"]],(some "title")⟩],[]⟩
def nativeContent_122 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,25,.title,.ordinary,"Old/[[New]] [[X]]","Old/[[New]] [[X]]"⟩ [.inl "Old/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩,.inl " ",.inr ⟨20,25,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_123 : Content := ⟨[],[⟨0,[.text "child"],false⟩]⟩
def nativeContent_124 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,25,.title,.ordinary,"New/[[New]] [[X]]","New/[[New]] [[X]]"⟩ [.inl "New/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩,.inl " ",.inr ⟨20,25,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_125 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"Old/[[Old]]","Old/[[Old]]"⟩ [.inl "Old/",.inr ⟨12,19,.link,.ordinary,"Old","[[Old]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_126 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"old/[[old]]","old/[[old]]"⟩ [.inl "old/",.inr ⟨12,19,.link,.ordinary,"old","[[old]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_127 : Content := ⟨[⟨.other,.markdown,[.text "alias::"],(some "alias")⟩],[]⟩
def nativeContent_128 : Content := ⟨[],[⟨0,[.text "source prose\n",.text "\n"],false⟩]⟩
def nativeContent_129 : Content := ⟨[],[⟨0,[.text "target prose\n",.text "\n"],false⟩]⟩
def nativeContent_130 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"A","A"⟩ [.inl "A"]],(some "title")⟩],[]⟩
def nativeContent_131 : Content := ⟨[],[⟨0,[.text "kid"],false⟩]⟩
def nativeContent_132 : Content := ⟨[],[⟨0,[.text "\n",.text "title:: B"],false⟩]⟩
def nativeContent_133 : Content := ⟨[],[⟨0,[.text "p"],false⟩]⟩
def nativeContent_134 : Content := ⟨[],[⟨0,[.text "q"],false⟩]⟩
def nativeContent_135 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,11,.title,.ordinary,"A/x","A/x"⟩ [.inl "A/x"]],(some "title")⟩],[]⟩
def nativeContent_136 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,9,.title,.ordinary,"C","C"⟩ [.inl "C"]],(some "title")⟩],[]⟩
def nativeContent_137 : Content := ⟨[],[⟨0,[.text "occupied"],false⟩]⟩
def nativeContent_138 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,19,.title,.ordinary,"New/[[New]]","New/[[New]]"⟩ [.inl "New/",.inr ⟨12,19,.link,.ordinary,"New","[[New]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_139 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,13,.title,.ordinary,"[[X]]","[[X]]"⟩ [.inr ⟨8,13,.link,.ordinary,"X","[[X]]"⟩]],(some "title")⟩],[]⟩
def nativeContent_140 : Content := ⟨[⟨.title,.markdown,[.text "title:: ",.title ⟨8,18,.title,.ordinary,"2026/10/26","2026/10/26"⟩ [.inl "2026/10/26"]],(some "title")⟩],[]⟩
def nativeContent_141 : Content := ⟨[],[⟨0,[.text "day"],false⟩]⟩
def nativeContent_142 : Content := ⟨[],[⟨0,[.site ⟨0,14,.link,.ordinary,"2026/10/26","[[2026/10/26]]"⟩,.text " ",.site ⟨15,25,.link,.ordinary,"2026/x","[[2026/x]]"⟩],false⟩]⟩
def nativeContent_143 : Content := ⟨[],[⟨0,[.site ⟨0,14,.link,.ordinary,"2026/10/26","[[2026/10/26]]"⟩,.text " ",.site ⟨15,22,.link,.ordinary,"B/x","[[B/x]]"⟩],false⟩]⟩
def nativeContent_144 : Content := ⟨[],[]⟩
def nativeContent_145 : Content := ⟨[],[⟨0,[.text "h ",.site ⟨2,7,.tag,.orgHeadlineTag,"New",":New:"⟩],false⟩]⟩
def nativeContent_146 : Content := ⟨[],[⟨0,[.site ⟨0,7,.link,.ordinary,"A/x","[[A/x]]"⟩],false⟩]⟩
def nativeContent_147 : Content := ⟨[],[⟨0,[.text "bx"],false⟩]⟩
def nativeContent_148 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,13,.aliasName,.ordinary,"A/sub","A/sub"⟩],(some "alias")⟩],[]⟩
def nativeContent_149 : Content := ⟨[],[⟨0,[.text "bs"],false⟩]⟩
def nativeContent_150 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,9,.aliasName,.ordinary,"Y","Y"⟩],(some "alias")⟩],[]⟩
def nativeContent_151 : Content := ⟨[⟨.aliases,.markdown,[.text "alias:: ",.site ⟨8,13,.aliasName,.ordinary,"B/sub","B/sub"⟩],(some "alias")⟩],[]⟩
def nativeContent_152 : Content := ⟨[⟨.property,.directive,[.text "#+",.site ⟨2,5,.propertyKey,.ordinary,"key","key"⟩,.text ": ",.site ⟨7,12,.link,.ordinary,"X","[[X]]"⟩],(some "key")⟩],[]⟩
def nativeContent_153 : Content := ⟨[⟨.property,.directive,[.text "#+",.site ⟨2,5,.propertyKey,.ordinary,"key","key"⟩,.text ": ",.text "target"],(some "key")⟩],[]⟩
def nativeContent_154 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"j","j"⟩,.text ":: ",.text "J"],(some "j")⟩],[]⟩
def nativeContent_155 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"k","k"⟩,.text ":: ",.text "K"],(some "k")⟩],[]⟩
def nativeContent_156 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"x","x"⟩,.text ":: ",.text "X"],(some "x")⟩],[]⟩
def nativeContent_157 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,1,.propertyKey,.ordinary,"y","y"⟩,.text ":: ",.text "Y"],(some "y")⟩],[]⟩
def nativeContent_158 : Content := ⟨[],[⟨0,[.text "\n",.text "#+BEGIN_QUOTE\n",.text "#+key: X\n",.text "#+END_QUOTE"],false⟩]⟩
def nativeContent_159 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.site ⟨6,11,.link,.ordinary,"X","[[X]]"⟩],(some "key")⟩],[]⟩
def nativeContent_160 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.text "target"],(some "key")⟩],[]⟩
def nativeContent_161 : Content := ⟨[],[⟨0,[.text "\n",.site ⟨1,4,.propertyKey,.ordinary,"key","key"⟩,.text ":: ",.site ⟨7,12,.link,.ordinary,"X","[[X]]"⟩],false⟩]⟩
def nativeContent_162 : Content := ⟨[],[⟨0,[.text "\n",.site ⟨1,4,.propertyKey,.ordinary,"key","key"⟩,.text ":: X"],false⟩]⟩
def nativeContent_163 : Content := ⟨[],[⟨0,[.text "\n",.text "#+BEGIN_QUOTE\n",.text "#+key: [[X]]\n",.text "#+END_QUOTE"],false⟩]⟩
def nativeContent_164 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "star"],(some "icon")⟩,⟨.property,.markdown,[.text "public",.text ":: ",.text "true"],(some "public")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_165 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "star"],(some "icon")⟩,⟨.property,.markdown,[.text "public",.text ":: ",.text "true"],(some "public")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_166 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "star"],(some "icon")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_167 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "moon"],(some "icon")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_168 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "moon"],(some "icon")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.text "icon:: star"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_169 : Content := ⟨[⟨.property,.markdown,[.text "public",.text ":: ",.text "true"],(some "public")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_170 : Content := ⟨[⟨.property,.markdown,[.text "public",.text ":: ",.text "true"],(some "public")⟩,⟨.property,.markdown,[.text "icon",.text ":: ",.text "star"],(some "icon")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_171 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "source"],(some "Old")⟩],[⟨0,[.text "source"],false⟩]⟩
def nativeContent_172 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,9,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "target"],(some "two-words")⟩],[⟨0,[.text "target"],false⟩]⟩
def nativeContent_173 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,9,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "target"],(some "two-words")⟩],[⟨0,[.text "target"],false⟩,⟨0,[.text "\n",.site ⟨33,42,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: source"],false⟩,⟨0,[.text "source"],false⟩]⟩
def nativeContent_174 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "star"],(some "icon")⟩],[]⟩
def nativeContent_175 : Content := ⟨[⟨.property,.markdown,[.text "public",.text ":: ",.text "true"],(some "public")⟩],[]⟩
def nativeContent_176 : Content := ⟨[⟨.property,.markdown,[.text "icon",.text ":: ",.text "moon"],(some "icon")⟩],[]⟩
def nativeContent_177 : Content := ⟨[],[⟨0,[.text "\n",.text "icon:: star"],false⟩]⟩
def nativeContent_178 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,3,.propertyKey,.ordinary,"Old","Old"⟩,.text ":: ",.text "source"],(some "Old")⟩],[]⟩
def nativeContent_179 : Content := ⟨[⟨.property,.markdown,[.site ⟨0,9,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: ",.text "target"],(some "two-words")⟩],[]⟩
def nativeContent_180 : Content := ⟨[],[⟨0,[.text "\n",.site ⟨1,10,.propertyKey,.ordinary,"two-words","two-words"⟩,.text ":: source"],false⟩]⟩
def nativeTable : List (Format × String × Content) := [
  (.md,"- notes\n",nativeContent_0),
  (.md,"- [[A]]\n",nativeContent_1),
  (.md,"- [[B]]\n",nativeContent_2),
  (.md,"- source\n",nativeContent_3),
  (.md,"- target\n",nativeContent_4),
  (.md,"- target\n- source\n",nativeContent_5),
  (.md,"clash:: source\njoin:: value\n- s\n",nativeContent_6),
  (.md,"clash:: target\n- t\n",nativeContent_7),
  (.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n",nativeContent_8),
  (.org,"#+title: A\n* source\n",nativeContent_9),
  (.org,"#+title: B\n* target\n",nativeContent_10),
  (.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n",nativeContent_11),
  (.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n",nativeContent_12),
  (.org,"* h :Old:\n",nativeContent_13),
  (.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n",nativeContent_14),
  (.md,"- #Old\n",nativeContent_15),
  (.md,"- #[[Two Words]]\n",nativeContent_16),
  (.md,"- target",nativeContent_17),
  (.md,"alias:: C#\n- source\n",nativeContent_18),
  (.md,"alias:: C\\#\n- target\n",nativeContent_19),
  (.md,"alias:: C\\#\n- target\n- source\n",nativeContent_20),
  (.md,"- Old:: value\n- Old:: another\n",nativeContent_21),
  (.md,"- two-words:: value\n- two-words:: another\n",nativeContent_22),
  (.md,"alias:: X\n- source\n",nativeContent_23),
  (.md,"title:: X\n- source\n",nativeContent_24),
  (.md,"title:: X\n- target\n",nativeContent_25),
  (.md,"title:: X\n- target\n- \n  title:: X\n- source\n",nativeContent_26),
  (.md,"x:: one\nx:: two\n- source\n",nativeContent_27),
  (.md,"x:: one\n- target\n- \n  x:: two\n- source\n",nativeContent_28),
  (.md,"alias:: A\n- owner\n",nativeContent_29),
  (.md,"alias:: B\n- owner\n",nativeContent_30),
  (.md,"title:: 2026-11-02\n- target\n",nativeContent_31),
  (.md,"title:: 2026-11-02\n- target\n- source\n",nativeContent_32),
  (.md,"title:: Old/[[New]] [[X]]\n- child\n",nativeContent_33),
  (.md,"- target\n- notes\n",nativeContent_34),
  (.md,"title:: New/[[New]] [[X]]\n- child\n",nativeContent_35),
  (.md,"title:: Old/[[Old]]\n- child\n",nativeContent_36),
  (.md,"title:: old/[[old]]\n- child\n",nativeContent_37),
  (.md,"alias::\n- target\n",nativeContent_38),
  (.md,"alias::\n- target\n- source\n",nativeContent_39),
  (.md,"source prose\n\n- source\n",nativeContent_40),
  (.md,"target prose\n\n- target\n",nativeContent_41),
  (.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n",nativeContent_42),
  (.md,"title:: A\n- alias:: X\n  - kid\n- source\n",nativeContent_43),
  (.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n",nativeContent_44),
  (.md,"alias:: X\n- owner\n",nativeContent_45),
  (.md,"title:: X\n- p\n",nativeContent_46),
  (.md,"title:: X\n- q\n",nativeContent_47),
  (.md,"title:: A/x\n- p\n",nativeContent_48),
  (.md,"title:: A/x\n- q\n",nativeContent_49),
  (.md,"title:: C\n- occupied\n",nativeContent_50),
  (.md,"- alias:: X\n  - kid\n- source\n",nativeContent_51),
  (.md,"title:: New/[[New]]\n- child\n",nativeContent_52),
  (.md,"title:: [[X]]\n- notes\n",nativeContent_53),
  (.md,"title:: 2026/10/26\n- day\n",nativeContent_54),
  (.md,"- [[2026/10/26]] [[2026/x]]\n",nativeContent_55),
  (.md,"- [[2026/10/26]] [[B/x]]\n",nativeContent_56),
  (.md,"join:: value\n- s\n",nativeContent_57),
  (.md,"join:: value\nclash:: target\n- t\n- s\n",nativeContent_58),
  (.md,"\n- source\n",nativeContent_59),
  (.md,"- \n- source\n",nativeContent_60),
  (.org,"* h :New:\n",nativeContent_61),
  (.md,"- [[A/x]]\n",nativeContent_62),
  (.md,"- bx\n",nativeContent_63),
  (.md,"alias:: A/sub\n- owner\n",nativeContent_64),
  (.md,"- bs\n",nativeContent_65),
  (.md,"- child\n",nativeContent_66),
  (.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n",nativeContent_67),
  (.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n",nativeContent_68),
  (.md,"alias:: Y\n- target\n",nativeContent_69),
  (.md,"alias:: Y\nalias:: X\n- target\n- source\n",nativeContent_70),
  (.md,"alias:: B/sub\n- source\n",nativeContent_71),
  (.org,"#+key: [[X]]\n* source\n",nativeContent_72),
  (.org,"#+key: target\n* target\n",nativeContent_73),
  (.md,"j:: J\nk:: K\n- source\n",nativeContent_74),
  (.md,"x:: X\ny:: Y\n- target\n",nativeContent_75),
  (.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n",nativeContent_76),
  (.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n",nativeContent_77),
  (.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n",nativeContent_78),
  (.md,"key:: [[X]]\n- source\n",nativeContent_79),
  (.md,"key:: target\n- target\n",nativeContent_80),
  (.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n",nativeContent_81),
  (.md,"key:: target\n- target\n- \n  key:: X\n- source\n",nativeContent_82),
  (.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n",nativeContent_83),
  (.md,"notes",nativeContent_84),
  (.md,"[[A]]",nativeContent_85),
  (.md,"[[B]]",nativeContent_86),
  (.md,"source",nativeContent_87),
  (.md,"target",nativeContent_88),
  (.md,"clash:: source",nativeContent_89),
  (.md,"join:: value",nativeContent_90),
  (.md,"s",nativeContent_91),
  (.md,"clash:: target",nativeContent_92),
  (.md,"t",nativeContent_93),
  (.md,"\nclash:: source",nativeContent_94),
  (.org,"#+title: A",nativeContent_95),
  (.org,"source",nativeContent_96),
  (.org,"#+title: B",nativeContent_97),
  (.org,"target",nativeContent_98),
  (.org,"\n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE",nativeContent_99),
  (.md,"<<<<<<< a\n=======\n>>>>>>> b\n",nativeContent_100),
  (.md,"[[Old]]",nativeContent_101),
  (.org,"h :Old:",nativeContent_102),
  (.org,"<<<<<<< a\n=======\n>>>>>>> b\n",nativeContent_103),
  (.md,"#Old",nativeContent_104),
  (.md,"#[[Two Words]]",nativeContent_105),
  (.md,"alias:: C#",nativeContent_106),
  (.md,"alias:: C\\#",nativeContent_107),
  (.md,"Old:: value",nativeContent_108),
  (.md,"Old:: another",nativeContent_109),
  (.md,"two-words:: value",nativeContent_110),
  (.md,"two-words:: another",nativeContent_111),
  (.md,"alias:: X",nativeContent_112),
  (.md,"title:: X",nativeContent_113),
  (.md,"\ntitle:: X",nativeContent_114),
  (.md,"x:: one",nativeContent_115),
  (.md,"x:: two",nativeContent_116),
  (.md,"\nx:: two",nativeContent_117),
  (.md,"alias:: A",nativeContent_118),
  (.md,"owner",nativeContent_119),
  (.md,"alias:: B",nativeContent_120),
  (.md,"title:: 2026-11-02",nativeContent_121),
  (.md,"title:: Old/[[New]] [[X]]",nativeContent_122),
  (.md,"child",nativeContent_123),
  (.md,"title:: New/[[New]] [[X]]",nativeContent_124),
  (.md,"title:: Old/[[Old]]",nativeContent_125),
  (.md,"title:: old/[[old]]",nativeContent_126),
  (.md,"alias::",nativeContent_127),
  (.md,"source prose\n\n",nativeContent_128),
  (.md,"target prose\n\n",nativeContent_129),
  (.md,"title:: A",nativeContent_130),
  (.md,"kid",nativeContent_131),
  (.md,"\ntitle:: B",nativeContent_132),
  (.md,"p",nativeContent_133),
  (.md,"q",nativeContent_134),
  (.md,"title:: A/x",nativeContent_135),
  (.md,"title:: C",nativeContent_136),
  (.md,"occupied",nativeContent_137),
  (.md,"title:: New/[[New]]",nativeContent_138),
  (.md,"title:: [[X]]",nativeContent_139),
  (.md,"title:: 2026/10/26",nativeContent_140),
  (.md,"day",nativeContent_141),
  (.md,"[[2026/10/26]] [[2026/x]]",nativeContent_142),
  (.md,"[[2026/10/26]] [[B/x]]",nativeContent_143),
  (.md,"",nativeContent_144),
  (.org,"h :New:",nativeContent_145),
  (.md,"[[A/x]]",nativeContent_146),
  (.md,"bx",nativeContent_147),
  (.md,"alias:: A/sub",nativeContent_148),
  (.md,"bs",nativeContent_149),
  (.md,"alias:: Y",nativeContent_150),
  (.md,"alias:: B/sub",nativeContent_151),
  (.org,"#+key: [[X]]",nativeContent_152),
  (.org,"#+key: target",nativeContent_153),
  (.md,"j:: J",nativeContent_154),
  (.md,"k:: K",nativeContent_155),
  (.md,"x:: X",nativeContent_156),
  (.md,"y:: Y",nativeContent_157),
  (.org,"\n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE",nativeContent_158),
  (.md,"key:: [[X]]",nativeContent_159),
  (.md,"key:: target",nativeContent_160),
  (.md,"\nkey:: [[X]]",nativeContent_161),
  (.md,"\nkey:: X",nativeContent_162),
  (.org,"\n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE",nativeContent_163),
  (.md,"icon:: star\npublic:: true\n- source\n",nativeContent_164),
  (.md,"icon:: star\npublic:: true\n- target\n- source\n",nativeContent_165),
  (.md,"icon:: star\n- source\n",nativeContent_166),
  (.md,"icon:: moon\n- target\n",nativeContent_167),
  (.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n",nativeContent_168),
  (.md,"public:: true\n- target\n",nativeContent_169),
  (.md,"public:: true\nicon:: star\n- target\n- source\n",nativeContent_170),
  (.md,"Old:: source\n- source\n",nativeContent_171),
  (.md,"two-words:: target\n- target\n",nativeContent_172),
  (.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n",nativeContent_173),
  (.md,"icon:: star",nativeContent_174),
  (.md,"public:: true",nativeContent_175),
  (.md,"icon:: moon",nativeContent_176),
  (.md,"\nicon:: star",nativeContent_177),
  (.md,"Old:: source",nativeContent_178),
  (.md,"two-words:: target",nativeContent_179),
  (.md,"\ntwo-words:: source",nativeContent_180)]
def nativeRead (fmt : Format) (s : String) : Content :=
  ((nativeTable.find? (fun x => x.1 == fmt && x.2.1 == s)).map (fun x => x.2.2)).getD ⟨[],[]⟩
def o : Observations := {
  read := nativeRead,
  conflict := fun f => f.text.startsWith "<<<<<<<", spell := spell,
  keySpelling := fun s => some (if s = "Two Words" then "two-words" else s),
  save := id, supported := fun f => nativeTable.any (fun x => x.1 == f.format && x.2.1 == f.text),
  stem := fun f =>
    if f.path = "pages/1.md" then "1" else
    if f.path = "pages/2.md" then "2" else
    if f.path = "pages/2026.md" then "2026" else
    if f.path = "pages/A%2Fsub.md" then "A/sub" else
    if f.path = "pages/A.md" then "A" else
    if f.path = "pages/A.org" then "A" else
    if f.path = "pages/B%2Fsub.md" then "B/sub" else
    if f.path = "pages/B%2Fx.md" then "B/x" else
    if f.path = "pages/B.md" then "B" else
    if f.path = "pages/B.org" then "B" else
    if f.path = "pages/C.md" then "C" else
    if f.path = "pages/Conflict.md" then "Conflict" else
    if f.path = "pages/New%2F[[New]] [[X]].md" then "New/[[New]] [[X]]" else
    if f.path = "pages/New%2F[[Old]].md" then "New/[[Old]]" else
    if f.path = "pages/New.md" then "New" else
    if f.path = "pages/Old.md" then "Old" else
    if f.path = "pages/Org.org" then "Org" else
    if f.path = "pages/OrgConflict.org" then "OrgConflict" else
    if f.path = "pages/P.md" then "P" else
    if f.path = "pages/Q.md" then "Q" else
    if f.path = "pages/R.md" then "R" else
    if f.path = "pages/T.md" then "T" else
    if f.path = "pages/Tag.md" then "Tag" else
    if f.path = "pages/Two Words.md" then "Two Words" else
    if f.path = "pages/X.md" then "X" else
    if f.path = "pages/b.md" then "b" else
    if f.path = "pages/badchild.md" then "badchild" else
    if f.path = "pages/child.md" then "child" else
    if f.path = "pages/childcase.md" then "childcase" else
    if f.path = "pages/dayphysical.md" then "dayphysical" else
    if f.path = "pages/old%2F[[Old]].md" then "old/[[Old]]" else
    if f.path = "pages/old.md" then "old" else
    if f.path = "pages/physical.md" then "physical" else
    f.path,
  parent := fun _ => "pages/", extension := fun f => if f.format = .org then ".org" else ".md",
  encode := fun s => String.ofList (s.toList.flatMap (fun c => if c == '%' then ['%','2','5'] else if c == '/' then ['%','2','F'] else [c])), foldedPath := asciiLower }
def p := readingInputs o asciiLower (fun _ => false)
def isDone : Outcome → Bool | .success _ => true | _ => false
def rowOccurrences (t : Text) := (textSites t).filterMap (fun s => (occKind s.kind).map (·,s.name))
def fragmentLaw (fmt : Format) (c : Content) : Bool :=
  (contentRows c).all (fun row => decide (rowOccurrences row =
    (contentSites (nativeRead fmt (textString row))).filterMap (fun s => (occKind s.kind).map (·,s.name))))
def titlePartsLaw (c : Content) : Bool :=
  (c.header.flatMap Header.text ++ c.blocks.flatMap Block.body).all (fun a => match a with
    | .title s ps => decide (String.join (ps.map partText) = s.native) &&
        ps.all (fun a => match a with | .inl _ => true | .inr t => decide (s.start ≤ t.start ∧ t.stop ≤ s.stop))
    | _ => true)

end CutExamples
end CM
