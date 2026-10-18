// GH #453: global code wrapping preserves every mounted reading viewport.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.join(root, "artifacts/code-wrap-toggle");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:ui/toggle-code-block-wrap "mod+alt+w"}}\n');
const file = path.join(graph, "pages/Wrap Demo.md");
// All code and reading markers are descendants of one enclosing outline
// parent: its top stays fixed while wrapping moves the visible descendant.
const source = '- Long outline parent\n' + Array.from({ length: 16 }, (_, n) =>
  `  - \`\`\`js\n    ${Array.from({ length: 3 }, () => '// ' + 'long code line '.repeat(28)).join("\n    ")}\n    \`\`\`\n  - Reading marker ${n}\n`).join("");
fs.writeFileSync(file, source);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4876);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), workingDiff: true,
  journey: "Palette and bound wrap toggle preserve main, split and sidebar reading offsets in both directions", offsets: [] };
let browser;
let stage = "startup";
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "code-wrap-toggle") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Wrap Demo");
  stage = "mount split and sidebar";
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content").length === 2), { timeout: 10000 });
  await browser.execute(() => document.querySelector("h1.page-title").dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true })));
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".right-sidebar [data-block-id]")), { timeout: 10000 });
  await browser.execute(() => {
    for (const scroller of document.querySelectorAll(".main-content, .right-sidebar")) {
      const marker = [...scroller.querySelectorAll("[data-block-id]")].find(el => el.textContent.trim() === "Reading marker 6");
      if (!marker) throw Error("Reading marker absent");
      scroller.scrollTop += marker.getBoundingClientRect().top - scroller.getBoundingClientRect().top - 4;
    }
  });
  const offsets = () => browser.execute(() => [...document.querySelectorAll(".main-content, .right-sidebar")].map(scroller => {
    const marker = [...scroller.querySelectorAll("[data-block-id]")].find(el => el.textContent.trim() === "Reading marker 6");
    return marker.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  }));
  const before = await offsets(); proof.offsets.push(before);
  for (const wrapped of [true, false]) {
    stage = `toggle ${wrapped ? "on via palette" : "off via binding"}`;
    if (wrapped) {
      await browser.keys(["Control", "Shift", "p"]);
      const input = await browser.$(".switcher-input"); await input.waitForDisplayed(); await input.setValue("Toggle code block wrap");
      const row = await browser.$(".switcher-row*=Toggle code block wrap"); await row.waitForDisplayed(); await row.click();
    } else await browser.keys(["Control", "Alt", "w"]);
    await browser.waitUntil(() => browser.execute(expected => [...document.querySelectorAll("pre.code-block")]
      .every(el => el.classList.contains("code-wrapping") === expected), wrapped), { timeout: 10000 });
    await sleep(250);
    const after = await offsets(); proof.offsets.push(after);
    if (after.length !== 3 || after.some((offset, i) => Math.abs(offset - before[i]) > 3)) throw Error(`Viewport moved: ${JSON.stringify({ before, after })}`);
    await browser.saveScreenshot(path.join(out, wrapped ? "wrapped.png" : "unwrapped.png"));
  }
  if (fs.readFileSync(file, "utf8") !== source) throw Error("Toggling changed note source");
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #453: ${out}`);
} catch (error) {
  try { await browser.saveScreenshot(path.join(out, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Three mounted reading markers retain offsets within 3px in both directions; source unchanged",
    observed: String(error.stack), classification: "ambiguous; inspect log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
