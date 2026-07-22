#!/usr/bin/env node

// Prove the real GTK/Tauri binary advertises the desktop-entry identity on the
// Wayland wire. X11 tests cannot observe xdg_toplevel.app_id, and the call is a
// silent no-op if it runs before GTK has created the xdg_toplevel.

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/release/tine"));
const out = path.resolve(
  process.env.E2E_ARTIFACT_DIR || path.join(root, "test-results/e2e/wayland-app-id"),
);
// Wayland's Unix socket path is limited to 107 bytes. Release runners place
// artifacts under a deliberately descriptive (and therefore long) directory,
// so keep only the ephemeral runtime socket in the system temp directory.
const runtime = path.join(process.env.TMPDIR || "/tmp", `tine-wl-${process.pid}`);
const dataHome = path.join(out, "data");
const socket = `tine-wayland-${process.pid}`;
const clientLog = path.join(out, "client.log");
const compositorLog = path.join(out, "weston.log");

if (process.platform !== "linux") throw new Error("Wayland app-id regression is Linux-only");
if (!fs.existsSync(app)) throw new Error(`production app binary not found: ${app}`);
fs.mkdirSync(out, { recursive: true });
fs.rmSync(runtime, { recursive: true, force: true });
fs.rmSync(dataHome, { recursive: true, force: true });
fs.mkdirSync(runtime, { recursive: true, mode: 0o700 });
fs.chmodSync(runtime, 0o700);
fs.mkdirSync(dataHome, { recursive: true });

const portableRoot = [
  process.env.TINE_WESTON_ROOT,
  path.join(root, "../.codex-deps/weston/root"),
  path.join(root, "../../.codex-deps/weston/root"),
  path.join(root, "../../.tmp/weston-root"),
]
  .filter(Boolean)
  .map((candidate) => path.resolve(candidate))
  .find((candidate) => fs.existsSync(path.join(candidate, "usr/bin/weston")));

const westonEnv = { ...process.env, XDG_RUNTIME_DIR: runtime };
let weston = process.env.WESTON || "weston";
if (portableRoot) {
  weston = path.join(portableRoot, "usr/bin/weston");
  const lib = path.join(portableRoot, "usr/lib/x86_64-linux-gnu");
  const westonLib = path.join(lib, "weston");
  westonEnv.LD_LIBRARY_PATH = [lib, westonLib, westonEnv.LD_LIBRARY_PATH]
    .filter(Boolean)
    .join(path.delimiter);
  westonEnv.WESTON_MODULE_MAP = [
    `headless-backend.so=${path.join(lib, "libweston-10/headless-backend.so")}`,
    `kiosk-shell.so=${path.join(westonLib, "kiosk-shell.so")}`,
  ].join(";");
}

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function waitFor(predicate, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await delay(50);
  }
  throw new Error(message);
}

async function stopGroup(child) {
  if (!child || child.exitCode !== null) return;
  const exited = () => new Promise((resolve) => {
    if (child.exitCode !== null) resolve();
    else child.once("exit", resolve);
  });
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
  await Promise.race([exited(), delay(1_000)]);
  if (child.exitCode === null) {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {}
    await Promise.race([exited(), delay(1_000)]);
  }
}

const compositor = spawn(
  weston,
  [
    "--backend=headless-backend.so",
    "--shell=kiosk-shell.so",
    `--socket=${socket}`,
    "--idle-time=0",
    `--log=${compositorLog}`,
  ],
  { cwd: root, env: westonEnv, detached: true, stdio: "ignore" },
);
let client;

try {
  await waitFor(
    () => fs.existsSync(path.join(runtime, socket)) || compositor.exitCode !== null,
    10_000,
    "headless Weston did not publish its socket",
  );
  if (compositor.exitCode !== null) {
    throw new Error(`headless Weston exited ${compositor.exitCode}:\n${fs.readFileSync(compositorLog, "utf8")}`);
  }

  const stdout = fs.openSync(path.join(out, "app.stdout.log"), "w");
  const stderr = fs.openSync(clientLog, "w");
  const appEnv = {
    ...process.env,
    XDG_RUNTIME_DIR: runtime,
    XDG_DATA_HOME: dataHome,
    WAYLAND_DISPLAY: socket,
    GDK_BACKEND: "wayland",
    WAYLAND_DEBUG: "client",
    TINE_GPU: "0",
  };
  client = spawn(process.env.DBUS_RUN_SESSION || "dbus-run-session", ["--", app], {
    cwd: root,
    env: appEnv,
    detached: true,
    stdio: ["ignore", stdout, stderr],
  });

  await waitFor(
    () =>
      (fs.existsSync(clientLog) && fs.readFileSync(clientLog, "utf8").includes('set_app_id("page.tine.Tine")')) ||
      client.exitCode !== null,
    20_000,
    "Tine never advertised page.tine.Tine on the Wayland wire",
  );
  fs.closeSync(stdout);
  fs.closeSync(stderr);
  const wire = fs.readFileSync(clientLog, "utf8");
  if (client.exitCode !== null && !wire.includes('set_app_id("page.tine.Tine")')) {
    throw new Error(`Tine exited ${client.exitCode} before advertising its app ID:\n${wire.slice(-4_000)}`);
  }

  const fallbackAt = wire.indexOf('set_app_id("tine")');
  const tineAt = wire.indexOf('set_app_id("page.tine.Tine")');
  const firstBufferAt = wire.indexOf(".attach(", fallbackAt);
  if (fallbackAt < 0 || tineAt < fallbackAt) {
    throw new Error("Wayland trace does not show Tine overriding GTK's executable-name fallback");
  }
  if (firstBufferAt >= 0 && tineAt > firstBufferAt) {
    throw new Error("Tine advertised its application ID only after the first visible buffer");
  }

  const desktop = path.join(dataHome, "applications/page.tine.Tine.desktop");
  if (!fs.existsSync(desktop) || !fs.readFileSync(desktop, "utf8").includes("Icon=page.tine.Tine")) {
    throw new Error("standalone release binary did not install the matching desktop entry");
  }

  fs.writeFileSync(
    path.join(out, "result.json"),
    `${JSON.stringify({ app, appId: "page.tine.Tine", fallbackOverridden: true, beforeFirstBuffer: true }, null, 2)}\n`,
  );
  console.log("Wayland app ID OK: page.tine.Tine before the first visible buffer");
} finally {
  await stopGroup(client);
  await stopGroup(compositor);
  try {
    fs.rmSync(runtime, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  } catch (error) {
    // The runtime is unique to this process and contains no user data. A late
    // portal helper can briefly recreate an entry during shutdown; cleanup must
    // not turn a successful wire-level identity proof into an application failure.
    console.warn(`Wayland runtime cleanup deferred: ${error}`);
  }
}
