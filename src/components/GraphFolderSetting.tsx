import { settingLabel, settingAttributes } from "./settingsField";
import { settings } from "./settingDefinitions";
import { Show, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphMeta } from "../graphSession";
import { graphOwner, readOwned } from "../owned";
import { canRevealInFileManager } from "../nativeChrome";
import { pushToast } from "../toasts";
import { msg, t } from "../i18n";

/** Settings → Graph: where the open graph lives on disk (GH #447). The path is
 *  shown on every platform, in the backend's presentable spelling (`display_root`:
 *  on iOS the folder inside Tine's Documents, never the sandbox prefix, GH #926); **Show in folder** reveals the graph root in the
 *  system file manager where the build can (desktop), through the same native
 *  command as the graph menu's row action. Read-only: nothing is written. */
export function GraphFolderSetting(): JSX.Element {
  const reveal = async (path: string) => {
    const owner = graphOwner();
    try {
      await readOwned(owner, backend().revealKnownGraph(path));
    } catch (error) {
      pushToast(msg("toast-graph-folder-reveal-failed", { error: String(error) }), "error");
    }
  };
  return (
    <Show when={graphMeta()?.root}>{(root) =>
      <div class="settings-field" {...settingAttributes(settings["settings-graph-folder-label"])}>
        <div class="settings-field-row" >
          <span class="settings-label">{settingLabel(settings["settings-graph-folder-label"])}</span>
          <div class="settings-field-control">
            <span class="settings-value" data-graph-folder-value>{graphMeta()?.display_root || root()}</span>
            <Show when={canRevealInFileManager()}>
              <button class="settings-btn" onClick={() => void reveal(root())}>{t("settings-graph-folder-reveal")}</button>
            </Show>
          </div>
        </div>
      </div>
    }</Show>
  );
}
