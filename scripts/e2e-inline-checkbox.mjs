// GH #691: real pointer click, guarded durable save, route reopen and collapse.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { setTimeout as delay } from "node:timers/promises";

ensurePrivateSessionBus();

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/inline-checkbox/native"));
fs.mkdirSync(OUT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(OUT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const fixtures = [
  { name: "Checklist", ext: "md", raw: "- Checklist:\n  * [ ] same\n     * [ ] same\n\t- Child block\n- untouched\n" },
  { name: "Org checklist", ext: "org", raw: "* Checklist:\n- [ ] same\n   + [ ] same\n** Child block\n* untouched\n" },
];
for (const f of fixtures) fs.writeFileSync(path.join(GRAPH, `pages/${f.name}.${f.ext}`), f.raw);
fs.writeFileSync(path.join(GRAPH, "pages/Other.md"), "- Other\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), app: APP,
  binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), observations: [] };
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("tauri-driver did not become ready");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  for (const f of fixtures) {
    await openPageByName(browser, f.name);
    const boxes = await browser.$$(".main-content .block-checkbox");
    if (boxes.length !== 2) throw new Error(`${f.ext}: expected two inline checkboxes`);
    const geometry = await browser.execute(() => {
      const boxes = [...document.querySelectorAll(".main-content .block-checkbox")];
      const block = boxes[0].closest(".ls-block");
      const parent = boxes[0].closest(".block-content").getBoundingClientRect();
      const child = block.querySelector(".block-children .block-content").getBoundingClientRect();
      return { listStep: boxes[1].getBoundingClientRect().left - boxes[0].getBoundingClientRect().left,
        outlineStep: child.left - parent.left };
    });
    if (geometry.listStep < geometry.outlineStep * 0.75) throw new Error(`${f.ext}: nested hierarchy nearly flush ${JSON.stringify(geometry)}`);
    await boxes[1].click();
    if ((await browser.$$("textarea.block-editor")).length) throw new Error("Checkbox entered edit mode");
    await browser.waitUntil(async () => (await browser.$$('.main-content .block-checkbox[aria-checked="true"]')).length === 1,
      { timeout: 5000, timeoutMsg: "Selected checkbox did not visibly tick" });
    const token = f.raw.lastIndexOf("[ ]");
    const checked = f.raw.slice(0, token) + "[x]" + f.raw.slice(token + 3);
    const file = path.join(GRAPH, `pages/${f.name}.${f.ext}`);
    await browser.waitUntil(() => fs.readFileSync(file, "utf8") === checked, { timeout: 15000, timeoutMsg: "Expected only the clicked token to change on disk" });
    await openPageByName(browser, "Other");
    await openPageByName(browser, f.name);
    if ((await browser.$$('.main-content .block-checkbox[aria-checked="true"]')).length !== 1) throw new Error("Reopen lost checked state");
    await browser.saveScreenshot(path.join(TMP, `${f.ext}-checked.png`));
    const id = await browser.execute(() => document.querySelector(".main-content .block-checkbox").closest(".ls-block").getAttribute("data-block-id"));
    await browser.$(`[data-block-id="${id}"] .collapse-toggle`).click();
    if ((await browser.$$(".main-content .block-checkbox")).length !== 2) throw new Error("Collapse hid the block's own inline content");
    const child = await browser.$(`[data-block-id="${id}"] .block-children .block-content`);
    if (await child.isDisplayed()) throw new Error("Collapse left child block visible");
    await browser.saveScreenshot(path.join(TMP, `${f.ext}-collapsed.png`));
    await browser.$(`[data-block-id="${id}"] .collapse-toggle`).click();
    await browser.$$(".main-content .block-checkbox").then(items => items[1].click());
    await browser.waitUntil(() => fs.readFileSync(file, "utf8") === f.raw, { timeout: 15000, timeoutMsg: "Untick/expand did not restore original file bytes" });
    if ((await browser.$$('.main-content .block-checkbox[aria-checked="true"]')).length) throw new Error("Untick left a checked item visible");
    proof.observations.push({ format: f.ext, geometry, outcome: "Nested click saves only its token; reopen retains it; collapse keeps own list and hides child; untick/expand restores exact bytes" });
  }
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect native log and DOM";
  if (browser) {
    await browser.saveScreenshot(path.join(TMP, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
