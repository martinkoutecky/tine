import fs from "node:fs";
import path from "node:path";

export function tauriCapabilities(application, session = "default", platform = process.platform) {
  const options = { application };
  if (platform === "win32") {
    const root = process.env.E2E_WEBVIEW_USER_DATA_ROOT;
    if (!root) throw new Error("Windows WebView2 E2E requires E2E_WEBVIEW_USER_DATA_ROOT");
    const userDataFolder = path.join(root, session.replaceAll(/[^A-Za-z0-9_-]/g, "-"));
    fs.mkdirSync(userDataFolder, { recursive: true });
    // EdgeDriver's implicit temporary UDF can leave DevToolsActivePort under an
    // EBWebView child that the driver does not discover.  Tauri-driver forwards
    // this documented WebView2 option, giving both sides one explicit location.
    options.webviewOptions = { userDataFolder };
  }
  return {
    browserName: "wry",
    "wdio:enforceWebDriverClassic": true,
    "tauri:options": options,
  };
}
