// Capture the graph binding for asset writes/opens before any await. Rust rejects
// stale bindings before touching a graph. The editor check separately prevents a
// completed write from inserting a reference or reclaiming focus in a new editor.
// Both checks are O(1); a stale binding rejects, while a stale editor reports a
// stored but uninserted asset through reportStaleAsset.
import { backend } from "./backend";
import { errorFamily } from "./errorFamily";
import { ownedWhen, writeOwned } from "./owned";
import { captureBinding, bindingCurrent, type Binding } from "./binding";
import { editingId, editingOwner, editingSurface } from "./editorController";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";

export interface AssetEditorToken {
  readonly binding: Binding;
  readonly graphRoot: string | undefined;
  readonly textarea: HTMLTextAreaElement;
  readonly editingBlockId: string | null;
  readonly editingBlockOwner: string | null;
  readonly editingBlockSurface: string | null;
}

export function captureAssetEditor(textarea: HTMLTextAreaElement): AssetEditorToken {
  return {
    binding: captureBinding(),
    graphRoot: graphMeta()?.root,
    textarea,
    editingBlockId: editingId(),
    editingBlockOwner: editingOwner(),
    editingBlockSurface: editingSurface(),
  };
}

export function assetEditorIsCurrent(token: AssetEditorToken, textarea: HTMLTextAreaElement, mounted: boolean): boolean {
  return bindingCurrent(token.binding)
    && token.graphRoot === graphMeta()?.root
    && mounted
    && textarea === token.textarea
    && token.textarea.isConnected
    && editingId() === token.editingBlockId
    && editingOwner() === token.editingBlockOwner
    && editingSurface() === token.editingBlockSurface;
}

export function reportStaleAsset(): void {
  pushToast("The asset was saved, but it was not inserted because the graph or block changed.", "info");
}

const graphName = (root: string) => root.split(/[\\/]/).filter(Boolean).pop() ?? root;
const nextBindingGeneration = async (used: number): Promise<number> => {
  // A graph switch in flight: Rust has rebound the window, the frontend not yet.
  for (let i = 0; i < 300; i++) {
    const current = backend().graphBindingGeneration?.() ?? 0;
    if (current !== used && current > 0) return current;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("stale-graph-binding");
};

/** og H1b: a native capture (photo, voice memo) is kept in the graph it was
 * started in, named explicitly, even when it finishes after a graph switch: its
 * cache file is the only copy, so it is never dropped and never lands in the
 * next graph (Rust: capture_target.rs). The window's CURRENT binding
 * authorizes the call. Returns the stored name for the caller to insert while
 * the capture's graph is still open here; otherwise tells the user where the
 * file went and returns null. */
export async function importCaptureToOrigin(token: AssetEditorToken, path: string, candidate: string): Promise<string | null> {
  const origin = token.graphRoot;
  // Without a named graph, only the intent-time binding may receive it.
  let generation = origin === undefined ? token.binding.backendGeneration : backend().graphBindingGeneration?.() ?? 0;
  let stored: string;
  for (;;) {
    try {
      // The target is named, not the window's binding: no later graph can retire it.
      const result = await writeOwned(ownedWhen(), backend().importNativeCapture(path, candidate, generation, origin));
      if (result.kind === "stale") return null;
      stored = result.value;
      break;
    } catch (error) {
      if (origin === undefined || errorFamily(error) !== "stale-graph-binding") throw error;
      generation = await nextBindingGeneration(generation);
    }
  }
  if (origin === undefined || origin === graphMeta()?.root) return stored;
  pushToast(`Saved ${stored} to the assets/ of graph “${graphName(origin)}”; it was not inserted because the graph changed.`, "info");
  return null;
}
