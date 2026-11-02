// GH #794 Set depth 0 to 9 and GH #793 `:ref/default-open-blocks-level`, native.
// NOT YET RUN: written by the implementation lane; the manager runs native journeys.
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
import { editorSelector, scratchProfile } from "./lib/native-editor-bench.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/set-depth-794"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
// Set depth 2 and 0 are bound by the user; the reference level reaches every row of a four-level chain.
fs.writeFileSync(path.join(graph, "logseq/config.edn"),
  '{:shortcuts {:editor/set-depth-2 "mod+alt+2" :editor/set-depth-0 "mod+alt+0"} :ref/default-open-blocks-level 9 :tine/guide-announced? true}\n');
// Top (folded) > Kid (folded) > Grand (open) > Great (folded on purpose, hidden by Grand once it folds) > Leaf
const depthFile = path.join(graph, "pages", "Depth Demo.md");
fs.writeFileSync(depthFile,
  "- Top\n  collapsed:: true\n  - Kid\n    collapsed:: true\n    - Grand\n      - Great\n        collapsed:: true\n        - Leaf\n");
// #793: a hit with three levels below it, on a page that links to Target.
const sourceFile = path.join(graph, "pages", "Ref Source.md");
const sourceText = "- Hit [[Ref Target]]\n  - Ref child\n    - Ref grand\n      - Ref deep\n";
fs.writeFileSync(sourceFile, sourceText);
fs.writeFileSync(path.join(graph, "pages", "Ref Target.md"), "- Target body\n");
const env = { ...process.env, ...scratchProfile(out), TINE_GRAPH: graph,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4876);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Bound Set depth 2 opens the block and its child and folds the grandchild, leaving the folded great-grandchild alone; bound Set depth 0 folds the block; a high :ref/default-open-blocks-level opens every linked-reference row without writing the source file" };
// The block's own property lines, up to its first child.
const ownProps = (text, head) => {
  const lines = text.split("\n"), at = lines.indexOf(head), indent = head.indexOf("-");
  const own = [];
  for (let i = at + 1; i < lines.length && !lines[i].trimStart().startsWith("-") && lines[i].search(/\S/) > indent; i++) own.push(lines[i].trim());
  return at < 0 ? null : own;
};
const folded = (text, head) => ownProps(text, head)?.includes("collapsed:: true");
const shown = (browser, text) => browser.execute((text) => [...document.querySelectorAll(".block-content-wrapper")]
  .some(el => el.textContent.trim() === text && el.offsetParent), text);
const waitShown = (browser, text, expected) => browser.waitUntil(async () => (await shown(browser, text)) === expected,
  { timeout: 10000, timeoutMsg: `${text}: expected visible=${expected}` });
let browser;
let stage = "startup";
try {
  await sleep(1500);
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "set-depth") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });

  stage = "GH #794 select Top";
  await openPageByName(browser, "Depth Demo");
  await waitShown(browser, "Top", true);
  const domId = await browser.execute(() => [...document.querySelectorAll("[data-block-id] > .block-main .block-content-wrapper")]
    .find(el => el.textContent.trim() === "Top").closest("[data-block-id]").getAttribute("data-block-id"));
  await browser.$(`[data-block-id="${domId}"] > .block-main .block-content-wrapper`).click();
  await browser.$(`[data-block-id="${domId}"] > .block-main ${editorSelector}`).waitForDisplayed({ timeout: 10000 });
  await browser.keys(["Escape"]);
  stage = "Set depth 2";
  await browser.keys(["Control", "Alt", "2", "NULL"]);
  const two = await waitForFileText(depthFile, text => !folded(text, "- Top") && !folded(text, "  - Kid") && folded(text, "    - Grand"));
  if (!folded(two, "      - Great")) throw Error(`Set depth 2 changed the hidden great-grandchild: ${JSON.stringify(two)}`);
  await waitShown(browser, "Kid", true);
  await waitShown(browser, "Grand", true);
  await waitShown(browser, "Great", false);
  try { execFileSync("import", ["-window", "root", path.join(out, "depth-2.png")], { env }); } catch {}
  stage = "Set depth 0";
  await browser.keys(["Control", "Alt", "0", "NULL"]);
  const zero = await waitForFileText(depthFile, text => folded(text, "- Top"));
  await waitShown(browser, "Kid", false);
  if (folded(zero, "  - Kid") || !folded(zero, "    - Grand") || !folded(zero, "      - Great")) {
    throw Error(`Set depth 0 should only add Top's fold on top of Set depth 2's: ${JSON.stringify(zero)}`);
  }
  proof.depthTwo = two; proof.depthZero = zero;

  stage = "GH #793 linked references open to level 9";
  await openPageByName(browser, "Ref Target");
  for (const text of ["Hit", "Ref child", "Ref grand", "Ref deep"]) {
    await browser.waitUntil(() => browser.execute((text) => [...document.querySelectorAll(".block-content")]
      .some(el => el.textContent.includes(text) && el.offsetParent), text),
      { timeout: 15000, timeoutMsg: `reference row not shown: ${text}` });
  }
  try { execFileSync("import", ["-window", "root", path.join(out, "references-level-9.png")], { env }); } catch {}
  if (fs.readFileSync(sourceFile, "utf8") !== sourceText) throw Error("opening references by default level wrote the source file");
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #794/#793: ${out}`);
} catch (error) {
  try { execFileSync("import", ["-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, file: fs.readFileSync(depthFile, "utf8"),
    expected: "Set depth N opens levels above N and folds level N, leaving hidden deeper blocks alone; :ref/default-open-blocks-level opens references without writing files", observed: String(error.stack),
    classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
