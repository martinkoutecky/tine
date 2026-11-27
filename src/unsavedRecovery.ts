// Whether the unsaved-changes recovery panel is open (GH #540). A leaf module on
// purpose: the save engine opens the panel from a failure toast, so this must not
// import the document store. The panel belongs to the graph it was opened for; a
// graph switch closes it by retiring that binding.
import { createSignal } from "solid-js";
import { captureBinding, clearOnBindingInvalidated, bindingCurrent, graphScopedSignal, type Binding } from "./binding";

const [openFor, setOpenFor] = createSignal<Binding | null>(null);
export const unsavedRecoveryOpen = (): boolean => {
  const binding = openFor();
  return binding !== null && bindingCurrent(binding);
};
export const openUnsavedRecovery = (): void => { setOpenFor(captureBinding()); };
export const closeUnsavedRecovery = (): void => { setOpenFor(null); };
clearOnBindingInvalidated(closeUnsavedRecovery);

/** How many originals a cancelled rename kept in this graph (GH #540): told at
 * graph open (`DraftStatus.kept`) and by each list the panel takes. The palette
 * command that reopens the panel is offered while it is not zero. */
export const [keptOriginalCount, setKeptOriginalCount] = graphScopedSignal<number>("binding");
