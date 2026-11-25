// Rename sources whose original text a cancelled rename kept (the host's
// `DraftStatus.restorable`, refreshed by page mail when the list changes): the
// unsaved-changes recovery panel (GH #540) lists each with Restore and Copy. A
// leaf like `unsavedRecovery.ts`: graph open and the page-mail listener write
// it, the panel reads it. The list belongs to the graph binding it came from.
import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import type { RestorableSource } from "./document";
import { openUnsavedRecovery } from "./unsavedRecovery";

const [sources, setSources] = createSignal<readonly RestorableSource[]>([]);

/** The sources the panel offers to restore, in the host's order. */
export const restorableSources = (): readonly RestorableSource[] => sources();

/** The host's current list. `known`: the graph-open reply's list, which the
 * launch notice names; it never opens the panel. A list from page mail opens
 * the panel when it names a source not listed before (a Use Disk just
 * cancelled its rename); one that left (it was restored) only disappears. */
export function setRestorableSources(list: readonly RestorableSource[], known: boolean): void {
  const before = new Set(sources().map((source) => source.key));
  setSources(list);
  if (!known && list.some((source) => !before.has(source.key))) openUnsavedRecovery();
}

clearOnBindingInvalidated(() => setSources([]));
