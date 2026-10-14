import CustodyPublication
namespace Storage

theorem custodyResidual4_proved (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") : CustodyResidual4 c :=
  ⟨fun x ok hx h _ => custody_delivery c mutant x ok hx h,
   fun x hx h hen => custody_rename c mutant x hx h hen,
   fun x ok hx h hen => custody_dirSync c mutant x ok hx h hen,
   fun x p hx h hen => custody_draftSync c T texts mutant x hx h hen⟩

theorem custody_positiveTexts (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : Custody c x :=
  custody_positiveTexts_of_residual4 c T texts mutant (custodyResidual4_proved c T texts mutant) x hx

theorem noLoss_positiveTexts (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : noLoss x = true :=
  noLoss_of_custody c x (custody_positiveTexts c T texts mutant x hx)

#print axioms custodyResidual4_proved
#print axioms custody_positiveTexts
#print axioms noLoss_positiveTexts
end Storage
