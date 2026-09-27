// Capture the graph binding for asset writes/opens before any await. Rust rejects
// stale bindings before touching a graph. The editor check separately prevents a
// completed write from inserting a reference or reclaiming focus in a new editor.
// Both checks are O(1); a stale binding rejects, while a stale editor reports a
// stored but uninserted asset through reportStaleAsset.
import { captureBinding, stillBound, type Binding } from "./binding";
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
  return stillBound(token.binding)
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
