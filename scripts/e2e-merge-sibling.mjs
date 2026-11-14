// GH #764 and GH #818 (D-20261101-L5), in the real app, judged by the saved files:
//  - the palette command "Merge selected blocks" joins a block selection into its
//    first block (line breaks, children kept in order), and one Ctrl+Z restores it;
//  - Alt+Enter while editing adds a block at the same level after the block's whole
//    subtree, takes the typed text, and leaves the block it was pressed in alone.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-merge-sibling-764-818/native");
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const file = (name) => path.join(GRAPH, "pages", `${name}.md`);
const MERGE_SOURCE = "- alpha\n\t- alpha child\n- beta\n- gamma\n- delta\n";
const SIBLING_SOURCE = "- father 1\n\t- son\n\t\t- grandson\n\t- son 2\n- father 2\n";
fs.writeFileSync(file("Merge Demo"), MERGE_SOURCE);
fs.writeFileSync(file("Sibling Demo"), SIBLING_SOURCE);

const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4893);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "merge selected blocks and Alt+Enter sibling (GH #764, #818)", expected: "the saved files show the merged block and the new sibling after the subtree", steps: [] };

async function clickBlock(text) {
  const id = await browser.execute((t) => {
    const row = [...document.querySelectorAll(".main-content [data-block-id]")].find((el) => el.querySelector(".block-content-wrapper")?.textContent.trim() === t);
    if (!row) return null;
    row.scrollIntoView({ block: "center" });
    return row.getAttribute("data-block-id");
  }, text);
  if (!id) throw new Error(`no block reading ${JSON.stringify(text)}`);
  await browser.$(`.main-content [data-block-id="${id}"] .block-content-wrapper`).click();
  await browser.$(".main-content textarea.block-editor, .main-content [data-block-editor=\"cm6\"]").waitForExist({ timeout: 10_000 });
}

async function runPaletteCommand(label) {
  await browser.keys(["Control", "Shift", "p"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  await input.setValue(label);
  await browser.waitUntil(() => browser.execute((l) => [...document.querySelectorAll(".switcher-row")].some((el) => el.textContent.includes(l)), label), { timeout: 5000 });
  const ran = await browser.execute((l) => {
    const row = [...document.querySelectorAll(".switcher-row")].find((el) => el.textContent.includes(l));
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    return true;
  }, label);
  if (!ran) { await browser.keys("Escape"); throw new Error(`palette has no ${label}`); }
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await browser.execute(() => { for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click(); });

  // 1. Merge: Escape selects alpha's complete subtree; two Shift+Down presses
  //    add the beta and gamma sibling subtrees (GH #941).
  await openPageByName(browser, "Merge Demo");
  await clickBlock("alpha");
  await browser.keys("Escape");
  for (let i = 0; i < 2; i += 1) await browser.keys(["Shift", "ArrowDown"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .block-main.selected").length === 4), { timeout: 5000, timeoutMsg: "Shift+Down did not select four blocks" });
  await runPaletteCommand("Merge selected blocks");
  await waitForFileText(file("Merge Demo"), (body) => body === "- alpha\n  alpha child\n  beta\n  gamma\n- delta\n");
  proof.steps.push("merge: file is one block with the three lines, delta untouched");
  await browser.keys(["Control", "z"]);
  await waitForFileText(file("Merge Demo"), (body) => body === MERGE_SOURCE);
  proof.steps.push("merge: one Ctrl+Z restores the file exactly");

  // 2. Alt+Enter at a top-level block with children, then inside a nested one.
  await openPageByName(browser, "Sibling Demo");
  await clickBlock("father 1");
  await browser.keys(["Alt", "Enter"]);
  await browser.keys("new top");
  await browser.keys("Escape");
  await waitForFileText(file("Sibling Demo"), (body) => body === "- father 1\n\t- son\n\t\t- grandson\n\t- son 2\n- new top\n- father 2\n");
  proof.steps.push("Alt+Enter on father 1: the new block follows its whole subtree, before father 2, father 1 unsplit");
  await clickBlock("son");
  await browser.keys(["Alt", "Enter"]);
  await browser.keys("after son");
  await browser.keys("Escape");
  await waitForFileText(file("Sibling Demo"), (body) => body === "- father 1\n\t- son\n\t\t- grandson\n\t- after son\n\t- son 2\n- new top\n- father 2\n");
  proof.steps.push("Alt+Enter on the nested son: the new block follows its child, at its own level");

  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.observed = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    try { await browser.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  console.error("FAIL", JSON.stringify(proof));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
