import { documentOf, mainWindow } from "./windowRealm";

/** capture.html installs this root before its module script runs, so shared
 * imports can identify the scratch WebView synchronously, before mount/bind.
 * Its bounded completion binding is never a general graph-read capability. */
export function isQuickCaptureWindow(): boolean {
  const rootDocument = documentOf(mainWindow);
  return typeof rootDocument?.getElementById === "function" && rootDocument.getElementById("capture-root") !== null;
}

