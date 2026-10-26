// GH #696: native Collapse (one level) / Expand (one level) include the block itself.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/one-level-696"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"),
  '{:shortcuts {:editor/collapse-children "mod+alt+k" :editor/expand-children "mod+alt+j"}}\n');
const PARENT = "6960a000-0000-4000-8000-000000000696";
const file = path.join(graph, "pages", "Fold Demo.md");
fs.writeFileSync(file, `- Parent\n  id:: ${PARENT}\n  - Child\n    collapsed:: true\n    - Grand\n`);
// GH #768/#769: routed transclusions, kept separate from the original GH #696
// journey so a host fold cannot accidentally be mistaken for a source fold.
const embedSource = `- Embedded parent\n  id:: ${PARENT}\n  - Embedded child\n    - Embedded grandchild\n`;
const embedSourceFile = path.join(graph, "pages", "Embed Source.md");
// Use a distinct authored id from Fold Demo.
const EMBED = "7680a000-0000-4000-8000-000000000768";
fs.writeFileSync(embedSourceFile, embedSource.replace(PARENT, EMBED));
const embedHostFile = path.join(graph, "pages", "Embed Host.md");
fs.writeFileSync(embedHostFile, `- {{embed ((${EMBED}))}}\n  collapsed:: true\n- Page embed parent\n  - {{embed [[Page Body]]}}\n    collapsed:: true\n- [[Embed Host]]\n`);
fs.writeFileSync(path.join(graph, "pages", "Page Body.md"), "- Embedded page body\n");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4874);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Bound Collapse (one level) on a block whose only child is folded folds the block; bound Expand (one level) then opens only the block" };
// The block's own property lines, up to its first child.
const ownProps = (text, head) => {
  const lines = text.split("\n"), at = lines.indexOf(head), indent = head.indexOf("-");
  const own = [];
  for (let i = at + 1; i < lines.length && !lines[i].trimStart().startsWith("-") && lines[i].search(/\S/) > indent; i++) own.push(lines[i].trim());
  return at < 0 ? null : own;
};
const parentFolded = text => ownProps(text, "- Parent")?.includes("collapsed:: true");
const childFolded = text => ownProps(text, "  - Child")?.includes("collapsed:: true");
let browser;
let stage = "startup";
try {
  await sleep(1500);
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "one-level-fold") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Fold Demo");
  stage = "select Parent";
  // The DOM id is Tine's block id, not the authored `id::`; find the block by its text.
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll("[data-block-id] > .block-main .block-content-wrapper")]
    .some(el => el.textContent.trim() === "Parent")), { timeout: 10000, timeoutMsg: "Parent block did not render" });
  const domId = await browser.execute(() => [...document.querySelectorAll("[data-block-id] > .block-main .block-content-wrapper")]
    .find(el => el.textContent.trim() === "Parent").closest("[data-block-id]").getAttribute("data-block-id"));
  await browser.$(`[data-block-id="${domId}"] > .block-main .block-content-wrapper`).click();
  await browser.$(`[data-block-id="${domId}"] > .block-main textarea.block-editor`).waitForDisplayed({ timeout: 10000 });
  await browser.keys(["Escape"]);
  stage = "Collapse (one level) folds the block once its child is folded";
  await browser.keys(["Control", "Alt", "k"]);
  const folded = await waitForFileText(file, text => parentFolded(text) && childFolded(text));
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".block-content-wrapper") ||
    ![...document.querySelectorAll(".block-content-wrapper")].some(el => el.textContent.trim() === "Child" && el.offsetParent)),
    { timeout: 10000, timeoutMsg: "Child still visible after the block folded" });
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "folded.png")], { env }); } catch {}
  stage = "Expand (one level) opens only the block";
  await browser.keys(["Control", "Alt", "j"]);
  const opened = await waitForFileText(file, text => !parentFolded(text));
  if (!childFolded(opened)) throw Error(`Expand (one level) opened the child too: ${JSON.stringify(opened)}`);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".block-content-wrapper")]
    .some(el => el.textContent.trim() === "Child" && el.offsetParent) &&
    ![...document.querySelectorAll(".block-content-wrapper")].some(el => el.textContent.trim() === "Grand" && el.offsetParent)),
    { timeout: 10000, timeoutMsg: "Expected Child visible and Grand hidden after expanding the block" });
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "opened.png")], { env }); } catch {}
  proof.folded = folded; proof.opened = opened;
  stage = "GH #768 routed block embed";
  await openPageByName(browser, "Embed Host");
  const editRow = async (scope, text) => {
    await browser.waitUntil(() => browser.execute((scope, text) => [...document.querySelectorAll(`${scope} .block-content:not(.macro-host)`)]
      .some(el => el.textContent.trim() === text), scope, text), { timeout: 10000 });
    await browser.execute((scope, text) => {
      const el = [...document.querySelectorAll(`${scope} .block-content:not(.macro-host)`)].find(el => el.textContent.trim() === text);
      el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    }, scope, text);
    await browser.$(`${scope} textarea.block-editor`).waitForExist();
  };
  const visibleText = async (scope, text, expected) => browser.waitUntil(() => browser.execute((scope, text, expected) =>
    [...document.querySelectorAll(`${scope} .block-content`)].some(el => el.textContent.trim() === text && el.offsetParent) === expected,
    scope, text, expected), { timeout: 10000, timeoutMsg: `${text}: expected visible=${expected} in ${scope}` });
  const chord = async key => browser.keys(["Control", "Alt", key, "NULL"]);
  for (const scope of [".main-content", ".right-sidebar"]) {
    if (scope === ".right-sidebar") {
      await browser.execute(() => {
        const link = [...document.querySelectorAll(".main-content .page-ref")].find(el => el.textContent.includes("Embed Host"));
        link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
      });
      await browser.$(".right-sidebar .embed-block").waitForExist({ timeout: 10000 });
    }
    await editRow(scope, "Embedded parent");
    await chord("j");
    await visibleText(scope, "Embedded child", true);
    await waitForFileText(embedHostFile, text => text.includes("collapsed:: false"));
    await chord("k");
    await visibleText(scope, "Embedded grandchild", false);
    await browser.keys(["Escape"]); // retain the occurrence in selection mode
    await chord("j");
    await visibleText(scope, "Embedded grandchild", true);
    await chord("k"); await chord("k");
    await visibleText(scope, "Embedded child", false);
    await waitForFileText(embedHostFile, text => text.includes("collapsed:: true"));
    if (fs.readFileSync(embedSourceFile, "utf8") !== embedSource.replace(PARENT, EMBED)) throw Error("Embedded source was changed by occurrence folds");
    stage = `GH #769 child page embed in ${scope}`;
    await editRow(scope, "Page embed parent");
    await chord("j");
    await visibleText(scope, "Embedded page body", true);
    await chord("k");
    await visibleText(scope, "Embedded page body", false);
    await browser.keys(["Escape"]);
    // WebKit's WebDriver screenshot can stall with this software compositor;
    // use the same X11 capture as the original one-level journey above.
    execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, scope === ".main-content" ? "embeds-main.png" : "embeds-sidebar.png")], { env });
  }
  proof.embedSourceUnchanged = true;
  proof.outcome = "PASS";
  proof.journey += "; GH #768 block embed opens its own root then children; GH #769 child page embeds open and close, main pane and right sidebar";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #696: ${out}`);
} catch (error) {
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, file: fs.readFileSync(file, "utf8"),
    expected: "One-level folds follow displayed block/page embed state in main/sidebar and preserve embedded source folds", observed: String(error.stack),
    classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
