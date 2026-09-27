// An async asset write owns the graph and editor that started it. Capturing is
// O(1); checking is O(1). A stale completion keeps its stored bytes but cannot
// insert a reference or reclaim focus. There is no observable failure here;
// callers report a stale stored asset through reportStaleAsset.
import { editingId, editingOwner, editingSurface } from "./editorController";
import { graphEpoch, graphMeta } from "./graphSession";
import { pushToast } from "./toasts";

export interface AssetEditorToken {
  readonly graphEpoch: number;
  readonly graphRoot: string | undefined;
  readonly textarea: HTMLTextAreaElement;
  readonly editingBlockId: string | null;
  readonly editingBlockOwner: string | null;
  readonly editingBlockSurface: string | null;
}

export function captureAssetEditor(textarea: HTMLTextAreaElement): AssetEditorToken {
  return {
    graphEpoch: graphEpoch(),
    graphRoot: graphMeta()?.root,
    textarea,
    editingBlockId: editingId(),
    editingBlockOwner: editingOwner(),
    editingBlockSurface: editingSurface(),
  };
}

export function assetEditorIsCurrent(token: AssetEditorToken, textarea: HTMLTextAreaElement, mounted: boolean): boolean {
  return token.graphEpoch === graphEpoch()
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
