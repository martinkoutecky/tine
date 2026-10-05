/** Outline shell visibility, shared by all panes and model-driven navigation.
 * registerOutlineWindow borrows a list of descendant ids and a DOM host, and
 * returns its cleanup. O(ids) registration, no model/content mutation.
 * revealOutlineBlock synchronously mounts windows containing an id in the
 * optional view. Registry dispatch is O(instances of that id), plus synchronous
 * shell mounting, descendant registration and geometry; false means no match.
 * Callers still own ancestor expansion and scrolling. They need not know how
 * shells are grouped, estimated, mounted or measured.
 * listenForOutlinePrint owns browser print listeners for one rendered outline
 * or body and returns idempotent cleanup. O(1), no content access.
 * IDs must stay immutable until the idempotent cleanup runs; hosts belong to
 * that component lifetime. Graph invalidation clears registrations.
 * printingOutline is a readonly print-state getter. listenForOutlinePrint listens
 * to beforeprint/afterprint (including cancellation), rendering synchronously.
 * Static export reads the document independently of DOM.
 */
import { createRoot, createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";

type Window = { host: HTMLElement; reveal: () => void };
const windows = new Map<string, Set<Window>>();
const [printing, setPrintingOutline] = createRoot(() => createSignal(false));
export const printingOutline: () => boolean = printing;
let printWindow: typeof window | undefined;
const printOwners = new Set<object>();
const beforePrint = () => setPrintingOutline(true);
const afterPrint = () => setPrintingOutline(false);
function stopPrintListening(): void {
  printWindow?.removeEventListener("beforeprint", beforePrint);
  printWindow?.removeEventListener("afterprint", afterPrint);
  printWindow = undefined;
  setPrintingOutline(false);
}
clearOnBindingInvalidated(() => { windows.clear(); printOwners.clear(); stopPrintListening(); });

export function listenForOutlinePrint(): () => void {
  const owner = {};
  printOwners.add(owner);
  if (!printWindow && typeof window !== "undefined") {
    printWindow = window;
    window.addEventListener("beforeprint", beforePrint);
    window.addEventListener("afterprint", afterPrint);
  }
  return () => {
    printOwners.delete(owner);
    if (!printOwners.size) stopPrintListening();
  };
}

export function registerOutlineWindow(ids: readonly string[], host: HTMLElement, reveal: () => void): () => void {
  const entry = { host, reveal };
  for (const id of ids) {
    let entries = windows.get(id);
    if (!entries) windows.set(id, entries = new Set());
    entries.add(entry);
  }
  return () => {
    for (const id of ids) {
      const entries = windows.get(id);
      entries?.delete(entry);
      if (!entries?.size) windows.delete(id);
    }
  };
}

export function revealOutlineBlock(id: string, within?: Element | null): boolean {
  let found = false;
  // A parent reveal can register the nested window containing the same target.
  // Set iteration visits those new entries too, rendering the entire path.
  for (const entry of windows.get(id) ?? []) {
    if (within && !within.contains(entry.host)) continue;
    entry.reveal();
    found = true;
  }
  return found;
}
