// GH #640: a block opened in the right sidebar is titled with its page and ancestors, like OG.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/sidebar-crumbs-640"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const file = path.join(graph, "pages", "Crumb Demo.md");
fs.writeFileSync(file, "- First\n  - Second **bold**\n    - Third\n      - Fourth\n        - Leaf target\n");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4878);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Shift-click a nested block's bullet; the sidebar card title shows page, ellipsis and the nearest three ancestors rendered; clicking an ancestor crumb swaps the card to that block" };
const crumbs = () => browser.execute(() => [...document.querySelectorAll(".right-sidebar .rs-item")].map(item => ({
  title: item.querySelector(".rs-item-title")?.textContent, more: !!item.querySelector(".rs-crumb-more"),
  crumbs: [...item.querySelectorAll(".rs-crumb")].map(el => el.textContent), bold: item.querySelector(".rs-item-crumbs strong")?.textContent ?? null })));
let browser;
let stage = "startup";
try {
  await sleep(1500);
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "sidebar-crumbs") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Crumb Demo");
  stage = "shift-click the leaf bullet";
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll("[data-block-id] > .block-main .block-content-wrapper")]
    .some(el => el.textContent.trim() === "Leaf target")), { timeout: 10000, timeoutMsg: "Leaf block did not render" });
  const leafId = await browser.execute(() => [...document.querySelectorAll("[data-block-id] > .block-main .block-content-wrapper")]
    .find(el => el.textContent.trim() === "Leaf target").closest("[data-block-id]").getAttribute("data-block-id"));
  const bullet = await browser.$(`.main-content [data-block-id="${leafId}"] > .block-main .bullet-container`);
  await browser.performActions([{ type: "key", id: "kb", actions: [{ type: "keyDown", value: "\uE008" }] }]);
  await bullet.click();
  await browser.performActions([{ type: "key", id: "kb", actions: [{ type: "keyUp", value: "\uE008" }] }]);
  stage = "sidebar title shows page and ancestors";
  await browser.waitUntil(async () => (await crumbs()).some(c => c.crumbs.length === 3), { timeout: 10000, timeoutMsg: "no breadcrumb in the sidebar card" });
  const before = (await crumbs()).find(c => c.crumbs.length === 3);
  if (before.title !== "Crumb Demo" || !before.more || before.bold !== "bold" || before.crumbs.join("|") !== "Second bold|Third|Fourth")
    throw Error(`unexpected sidebar title: ${JSON.stringify(before)}`);
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "crumbs.png")], { env }); } catch {}
  stage = "click an ancestor crumb";
  await browser.execute(() => [...document.querySelectorAll(".right-sidebar .rs-crumb")].find(el => el.textContent === "Third").click());
  await browser.waitUntil(async () => { const c = await crumbs(); return c.length === 1 && c[0].crumbs.join("|") === "First|Second bold"; },
    { timeout: 10000, timeoutMsg: "crumb click did not swap the card to that block" });
  const after = await crumbs();
  if (after[0].more) throw Error(`ellipsis shown with only two ancestors: ${JSON.stringify(after)}`);
  if (fs.readFileSync(file, "utf8") !== "- First\n  - Second **bold**\n    - Third\n      - Fourth\n        - Leaf target\n") throw Error("navigation wrote the page");
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "swapped.png")], { env }); } catch {}
  proof.outcome = "PASS"; proof.before = before; proof.after = after;
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #640: ${out}`);
} catch (error) {
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Sidebar card titled page › ⋯ › three ancestors; crumb click swaps the card", observed: String(error.stack),
    classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
