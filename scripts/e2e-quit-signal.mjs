// GH #701: shutting down or logging out sends SIGTERM (and often SIGHUP right
// after) to every process of the session; closing a terminal sends SIGHUP and
// Ctrl+C sends SIGINT to the process group. Each must end Tine like Quit within
// a bound, removing the unclean-exit marker (ADR 0058), so the next launch does
// not say Tine "did not close cleanly". When only the app process is signalled
// (`kill <pid>`), its WebKit helpers survive and a just-typed edit must reach
// disk too. When the helpers die with it, the page cannot flush; the app must
// still exit and clear the marker. Before the fix the default action killed
// the process at once (exit status 143) and left the marker.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { APP_ID } from "./lib/app-identity.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver");
const ARTIFACT = path.join(ROOT, "artifacts/gh701/native");
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const PAGE = path.join(GRAPH, "pages", "Signal page.md");
fs.writeFileSync(PAGE, "- first line\n");
const MARKER = path.join(TMP, "data", APP_ID, "diagnostics", "session-active");

const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4895);
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "termination signals quit cleanly (GH #701)", expected: "SIGTERM and SIGHUP save the pending edit, exit, and remove the unclean-exit marker", steps: [] };

/** The app process this journey launched: the binary, with this journey's data home. */
function appPid() {
  for (const line of execFileSync("ps", ["-eo", "pid=,args="], { encoding: "utf8" }).split("\n")) {
    const match = /^\s*(\d+)\s+(\S+)/.exec(line);
    if (!match || path.resolve(match[2]) !== path.resolve(APP)) continue;
    try {
      if (fs.readFileSync(`/proc/${match[1]}/environ`, "utf8").split("\0").includes(`XDG_DATA_HOME=${env.XDG_DATA_HOME}`)) return Number(match[1]);
    } catch { /* exited meanwhile */ }
  }
  return null;
}
/** `pid` and every process below it (WebKit's helpers), as a session stop or a
 *  terminal's process-group signal reaches them all at once. */
function tree(pid) {
  const rows = execFileSync("ps", ["-eo", "pid=,ppid="], { encoding: "utf8" }).trim().split("\n").map((l) => l.trim().split(/\s+/).map(Number));
  const all = [pid];
  for (let i = 0; i < all.length; i++) for (const [child, parent] of rows) if (parent === all[i]) all.push(child);
  return all;
}
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };

/** `how` is "app" (only the app process), or a list of signals sent to the
 *  app and all its descendants in order. */
async function run(signal, text, how = "app") {
  const log = fs.openSync(path.join(TMP, `driver-${signal}.log`), "w");
  const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  try {
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
    await browser.$(".app-container").waitForExist({ timeout: 30_000 });
    if (!fs.existsSync(MARKER)) throw new Error(`${signal}: the running app holds no unclean-exit marker at ${MARKER}`);
    const pid = appPid();
    if (!pid) throw new Error(`${signal}: could not find the app process`);
    await openPageByName(browser, "Signal page");
    await browser.$(".main-content [data-block-id] .block-content-wrapper").click();
    await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 10_000 });
    await browser.keys("End");
    await browser.keys(text);
    // Within the save debounce: only the quit flush can save it.
    if (how === "app") process.kill(pid, signal);
    else {
      const all = tree(pid);
      for (const sig of how) for (const p of all) { try { process.kill(p, sig); } catch { /* gone */ } }
      proof.steps.push(`${signal}: sent ${how.join("+")} to ${all.length} processes (app and helpers)`);
    }
    const deadline = Date.now() + 30_000;
    while (alive(pid) && Date.now() < deadline) await sleep(100);
    if (alive(pid)) throw new Error(`${signal}: the app was still running 30 s after ${signal}`);
    proof.steps.push(`${signal}: the app exited`);
    if (how !== "app") {
      if (fs.existsSync(MARKER)) throw new Error(`${signal}: the unclean-exit marker survived, so the next launch reports that Tine did not close cleanly`);
      proof.steps.push(`${signal}: the unclean-exit marker was removed`);
      return;
    }
    await waitForFileText(PAGE, (body) => body.includes(text)).catch((error) => {
      throw new Error(`${signal}: the typed edit is not on disk (${error.message}); file is ${JSON.stringify(fs.readFileSync(PAGE, "utf8"))}`);
    });
    proof.steps.push(`${signal}: the pending edit ${JSON.stringify(text)} is on disk`);
    if (fs.existsSync(MARKER)) throw new Error(`${signal}: the unclean-exit marker survived, so the next launch reports that Tine did not close cleanly`);
    proof.steps.push(`${signal}: the unclean-exit marker was removed`);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

try {
  await run("SIGTERM", " saved by sigterm");
  await run("SIGHUP", " saved by sighup");
  await run("shutdown", " shutdown text", ["SIGTERM", "SIGHUP"]);
  await run("SIGINT", " ctrl-c text", ["SIGINT"]);
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  console.log(`e2e-quit-signal: PASS (${proof.steps.length} steps) ${TMP}`);
} catch (error) {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify({ ...proof, failure: String(error) }, null, 2)}\n`);
  console.error(`e2e-quit-signal: FAIL ${error.message}\n${TMP}`);
  process.exit(1);
}
