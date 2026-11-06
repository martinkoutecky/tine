import { dispatchFocusedEditorCommand } from "../editorCommandBridge";

/** The native menu can consume Cmd+Shift+V before JavaScript gets keydown.
 * Listen only on this webview window, so another graph never receives paste. */
export function installPlainPasteMenu(): () => void {
  let disposed = false;
  let unlisten = () => {};
  if ("__TAURI_INTERNALS__" in window) {
    void import("@tauri-apps/api/webviewWindow").then(async ({ getCurrentWebviewWindow }) => {
      const stop = await getCurrentWebviewWindow().listen("editor-paste-plain", () => {
        dispatchFocusedEditorCommand("editor/paste-plain");
      });
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => console.error("Could not listen for the native plain-text paste action"));
  }
  return () => { disposed = true; unlisten(); };
}
