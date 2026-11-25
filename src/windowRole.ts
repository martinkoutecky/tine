import { documentOf, mainWindow } from "./windowRealm";

/** capture.html installs this root before its module script runs, so shared
 * imports can identify the scratch WebView synchronously, before mount/bind.
 * Its bounded completion binding is never a general graph-read capability.
 *
 * The one answer to "may this window ask the graph": the backend gives Quick
 * Capture no graph slot (src-tauri `state.rs`) and refuses every graph read
 * there, so a module that reads the graph for what it shows (page names, page
 * icons, block references, asset bytes, reference counts) does not ask when
 * this is true, and answers as before its first answer arrives, raising no
 * error. Capture completion uses `capture_quick_switch` instead. O(1). */
export function isQuickCaptureWindow(): boolean {
  const rootDocument = documentOf(mainWindow);
  return typeof rootDocument?.getElementById === "function" && rootDocument.getElementById("capture-root") !== null;
}
