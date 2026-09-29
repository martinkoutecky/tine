// Whether the unsaved-changes recovery panel is open (GH #540). A leaf module on
// purpose: the save engine opens the panel from a failure toast, so this must not
// import the document store. The panel belongs to the graph it was opened for; a
// graph switch closes it by retiring that binding.
import { createSignal } from "solid-js";
import { captureBinding, clearOnBindingInvalidated, stillBound, type Binding } from "./binding";

const [openFor, setOpenFor] = createSignal<Binding | null>(null);
export const unsavedRecoveryOpen = (): boolean => {
  const binding = openFor();
  return binding !== null && stillBound(binding);
};
export const openUnsavedRecovery = (): void => { setOpenFor(captureBinding()); };
export const closeUnsavedRecovery = (): void => { setOpenFor(null); };
clearOnBindingInvalidated(closeUnsavedRecovery);
