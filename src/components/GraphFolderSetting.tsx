import { Show, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphMeta } from "../graphSession";
import { graphOwner, readOwned } from "../owned";
import { canRevealInFileManager } from "../nativeChrome";
import { pushToast } from "../toasts";

/** Settings → Graph: where the open graph lives on disk (GH #447). The path is
 *  shown on every platform; **Show in folder** reveals the graph root in the
 *  system file manager where the build can (desktop), through the same native
 *  command as the graph menu's row action. Read-only: nothing is written. */
export function GraphFolderSetting(): JSX.Element {
  const reveal = async (path: string) => {
    const owner = graphOwner();
    try {
      await readOwned(owner, backend().revealKnownGraph(path));
    } catch (error) {
      pushToast(`Could not show the graph folder: ${String(error)}`, "error");
    }
  };
  return (
    <Show when={graphMeta()?.root}>{(root) =>
      <div class="settings-field" data-setting-label="Graph folder">
        <div class="settings-field-row">
          <span class="settings-label">Graph folder</span>
          <div class="settings-field-control">
            <span class="settings-value" data-graph-folder-value>{root()}</span>
            <Show when={canRevealInFileManager()}>
              <button class="settings-btn" onClick={() => void reveal(root())}>Show in folder</button>
            </Show>
          </div>
        </div>
      </div>
    }</Show>
  );
}
