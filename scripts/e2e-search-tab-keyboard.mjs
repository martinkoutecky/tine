// GH #697: native Search tab typing, roving highlight, and keyboard navigation.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/keys-696-697"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:go/search-tab "mod+alt+f"}}\n');
for (const suffix of ["Alpha", "Beta", "Gamma"]) fs.writeFileSync(path.join(graph, "pages", `Keyboard ${suffix}.md`), `- Body for ${suffix}\n`);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4870);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Open Search tab through bound shortcut; type; ArrowDown then ArrowUp returns to the search box; ArrowDown twice; Enter opens second displayed page" };
let browser;
let stage = "startup";
try {
  await sleep(1500);
  // WebKitGTK permits one automation session. Another lane may own it briefly.
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "search-tab-keyboard") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await browser.$(".page-title").waitForDisplayed({ timeout: 30000 });
  stage = "open Search tab";
  await browser.keys(["Control", "Alt", "f"]);
  const input = await browser.$(".query-workspace-source");
  await input.waitForDisplayed({ timeout: 10000 });
  await input.click();
  for (const key of "Keyboard") await browser.keys([key]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace button[data-query-result]").length === 3),
    { timeout: 10000, timeoutMsg: "typed query did not return the three fixture pages" });
  const rows = await browser.execute(() => [...document.querySelectorAll(".query-workspace button[data-query-result]")]
    .map(row => row.querySelector(".search-result-excerpt")?.textContent));
  stage = "ArrowUp on the first row returns to the search box";
  await browser.keys(["ArrowDown"]);
  await browser.keys(["ArrowUp"]);
  const back = await browser.execute(() => ({ input: document.activeElement?.classList.contains("query-workspace-source"),
    active: document.querySelectorAll(".query-workspace button[data-query-result].active").length }));
  if (!back.input || back.active !== 0) throw Error(`ArrowUp on the first row did not return to the search box: ${JSON.stringify(back)}`);
  stage = "ArrowDown twice";
  await browser.keys(["ArrowDown"]);
  await browser.keys(["ArrowDown"]);
  const selected = await browser.execute(() => ({ text: document.activeElement?.querySelector(".search-result-excerpt")?.textContent,
    active: document.activeElement?.classList.contains("active"), tabIndex: document.activeElement?.tabIndex }));
  if (selected.text !== rows[1] || !selected.active || selected.tabIndex !== 0) throw Error(`Expected second accessible highlighted row: ${JSON.stringify({ rows, selected })}`);
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "highlight.png")], { env }); } catch {}
  stage = "Enter opens highlighted page";
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await browser.$(".page-title").getText()).trim() === rows[1],
    { timeout: 10000, timeoutMsg: "Enter did not open the highlighted page" });
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "opened.png")], { env }); } catch {}
  proof.outcome = "PASS"; proof.rows = rows; proof.opened = rows[1];
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #697: ${out}`);
} catch (error) {
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Second displayed page opens from Search tab after typing, ArrowDown twice and Enter", observed: String(error.stack),
    classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
