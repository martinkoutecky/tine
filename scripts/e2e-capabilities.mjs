import fs from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import path from "node:path";

export function tauriCapabilities(
  application,
  session = "default",
  platform = process.platform,
  debuggerAddress,
) {
  if (platform === "win32") {
    // When the app was started explicitly with a fixed remote-debugging port,
    // use EdgeDriver's documented WebView2 attach mode.  This deliberately
    // avoids its launch-mode DevToolsActivePort handshake, which is not
    // reliable on current hosted Windows/WebView2 runners.
    if (debuggerAddress) {
      return {
        browserName: "webview2",
        "wdio:enforceWebDriverClassic": true,
        "ms:edgeChromium": true,
        "ms:edgeOptions": { debuggerAddress },
      };
    }
    const root = process.env.E2E_WEBVIEW_USER_DATA_ROOT;
    if (!root) throw new Error("Windows WebView2 E2E requires E2E_WEBVIEW_USER_DATA_ROOT");
    const userDataFolder = path.join(root, session.replaceAll(/[^A-Za-z0-9_-]/g, "-"));
    fs.mkdirSync(userDataFolder, { recursive: true });
    return {
      browserName: "webview2",
      "wdio:enforceWebDriverClassic": true,
      "ms:edgeChromium": true,
      "ms:edgeOptions": {
        binary: application,
        args: [],
        // Give EdgeDriver and the hosted WebView one explicit writable profile
        // location. This is the native capability Tauri-driver would forward.
        webviewOptions: { userDataFolder },
      },
    };
  }
  return {
    browserName: "wry",
    "wdio:enforceWebDriverClassic": true,
    "tauri:options": { application },
  };
}

function windowsUserDataFolder(session) {
  const root = process.env.E2E_WEBVIEW_USER_DATA_ROOT;
  if (!root) throw new Error("Windows WebView2 E2E requires E2E_WEBVIEW_USER_DATA_ROOT");
  const userDataFolder = path.join(root, session.replaceAll(/[^A-Za-z0-9_-]/g, "-"));
  fs.mkdirSync(userDataFolder, { recursive: true });
  return userDataFolder;
}

function withFixedRemoteDebuggingPort(argumentsValue, port) {
  const withoutDynamicPort = String(argumentsValue || "")
    .replace(/(?:^|\s)--remote-debugging-port(?:=|\s+)\S+/g, " ")
    .trim();
  return [withoutDynamicPort, `--remote-debugging-port=${port}`].filter(Boolean).join(" ");
}

async function waitForDevTools(debuggerAddress, applicationProcess, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  const endpoint = `http://${debuggerAddress}/json/version`;
  let lastError = "endpoint not ready";
  while (Date.now() < deadline) {
    if (applicationProcess.exitCode !== null) {
      throw new Error(`WebView2 host exited before DevTools became ready (exit ${applicationProcess.exitCode})`);
    }
    try {
      const response = await fetch(endpoint, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) {
        const version = await response.json();
        if (version.webSocketDebuggerUrl) return version;
        lastError = `missing webSocketDebuggerUrl in ${JSON.stringify(version)}`;
      } else {
        lastError = `HTTP ${response.status}`;
      }
    } catch (error) {
      lastError = String(error);
    }
    await sleep(100);
  }
  throw new Error(`WebView2 DevTools did not become ready at ${endpoint}: ${lastError}`);
}

export async function startWebdriverApplication(
  application,
  env,
  debuggerPort,
  session = "default",
  platform = process.platform,
) {
  if (platform !== "win32") return { env, applicationProcess: undefined, debuggerAddress: undefined };

  const userDataFolder = windowsUserDataFolder(session);
  const debuggerAddress = `127.0.0.1:${debuggerPort}`;
  const applicationEnv = {
    ...env,
    TAURI_AUTOMATION: "true",
    TAURI_WEBVIEW_AUTOMATION: "true",
    WEBVIEW2_USER_DATA_FOLDER: userDataFolder,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: withFixedRemoteDebuggingPort(
      env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS,
      debuggerPort,
    ),
  };
  const applicationProcess = spawn(application, [], {
    env: applicationEnv,
    stdio: "ignore",
    windowsHide: false,
  });
  try {
    await waitForDevTools(debuggerAddress, applicationProcess);
  } catch (error) {
    stopWebdriverApplication({ applicationProcess }, platform);
    throw error;
  }
  return { env: applicationEnv, applicationProcess, debuggerAddress, userDataFolder };
}

export function stopWebdriverApplication(target, platform = process.platform) {
  const applicationProcess = target?.applicationProcess;
  if (!applicationProcess || applicationProcess.exitCode !== null) return;
  if (platform === "win32") {
    spawnSync("taskkill", ["/PID", String(applicationProcess.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    applicationProcess.kill("SIGKILL");
  }
}

export function webdriverServerArgs(port, nativePort, nativeDriver, platform = process.platform) {
  if (platform === "win32") return [`--port=${port}`];
  return [
    "--port", String(port),
    "--native-port", String(nativePort),
    "--native-driver", nativeDriver,
  ];
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
