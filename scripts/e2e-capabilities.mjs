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

function nestedActivePort(directory, depth = 0) {
  if (depth > 4 || !fs.existsSync(directory)) return undefined;
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const candidate = path.join(directory, entry.name);
    if (entry.isFile() && entry.name === "DevToolsActivePort") return candidate;
    if (entry.isDirectory()) {
      const found = nestedActivePort(candidate, depth + 1);
      if (found) return found;
    }
  }
  return undefined;
}

export function mirrorWindowsDevToolsActivePortOnce(root) {
  if (!fs.existsSync(root)) return 0;
  let mirrored = 0;
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const session = path.join(root, entry.name);
    const expected = path.join(session, "DevToolsActivePort");
    if (fs.existsSync(expected)) continue;
    const actual = nestedActivePort(session);
    if (!actual || actual === expected || fs.statSync(actual).size === 0) continue;
    // Current Edge/WebView2 releases can put this file in EBWebView/ while
    // EdgeDriver polls the configured UDF root.  Mirror, do not move: WebView2
    // continues to own its original profile layout.
    fs.copyFileSync(actual, expected);
    mirrored += 1;
  }
  return mirrored;
}

export function startWindowsDevToolsActivePortMirror(root, platform = process.platform) {
  if (platform !== "win32") return () => {};
  const timer = setInterval(() => {
    try {
      mirrorWindowsDevToolsActivePortOnce(root);
    } catch {
      // The WebView process creates and renames profile files concurrently.
      // A later poll gets a stable snapshot; the scenario timeout remains the
      // fail-closed boundary if no valid port ever appears.
    }
  }, 50);
  return () => clearInterval(timer);
}

export function windowsWebviewProfileSnapshot(root) {
  const files = [];
  function walk(directory, depth = 0) {
    if (depth > 6 || !directory || !fs.existsSync(directory)) return;
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      const relative = path.relative(root, absolute);
      try {
        if (entry.isDirectory()) {
          files.push({ path: `${relative}/`, type: "directory" });
          walk(absolute, depth + 1);
        } else if (entry.isFile()) {
          const stat = fs.statSync(absolute);
          const record = { path: relative, type: "file", size: stat.size };
          if (entry.name === "DevToolsActivePort") {
            record.contents = fs.readFileSync(absolute, "utf8").slice(0, 500);
          }
          files.push(record);
        }
      } catch (error) {
        files.push({ path: relative, type: "error", error: String(error) });
      }
    }
  }
  try {
    walk(root);
  } catch (error) {
    return { root, error: String(error), files };
  }
  return { root, files };
}
