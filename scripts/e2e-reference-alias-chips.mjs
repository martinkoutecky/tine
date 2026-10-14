// GH #433: native alias chips, legacy OG filters, guarded canonical save and
// restart. Fixtures and screenshots stay in this lane's ignored artifacts.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh433"));
fs.mkdirSync(ARTIFACTS, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACTS, "native-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Germany.md"), "alias:: DE, Deutschland\n\n- Canonical page\n");
const targetFile = path.join(GRAPH, "pages/Target.md");
fs.writeFileSync(targetFile, 'filters:: {"de" true}\n\n- Filter target\n');
fs.writeFileSync(path.join(GRAPH, "pages/Source.md"), [
  "- [[Target]] canonical [[Germany]]",
  "- [[Target]] short [[DE]]",
  "- [[Target]] translated [[Deutschland]]",
  "- [[Target]] both [[DE]] [[Deutschland]]",
  "- [[Target]] unrelated [[Zebra]]", "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, `journals/${journal}.md`), "- Open [[Target]]\n");
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(TMP, "xdg/data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg/config"), XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");

async function withApp(index, journey) {
  const port = Number(process.env.E2E_DRIVER_PORT || 4670) + index * 2;
  const log = fs.openSync(path.join(TMP, `driver-${index}.log`), "w");
  const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, detached: true, stdio: ["ignore", log, log] });
  driver.on("error", error => fs.writeFileSync(path.join(TMP, "driver-error.txt"), String(error)));
  let browser;
  try {
    await sleep(2500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
      connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
    await ensureMainWindow(browser);
    await browser.$(".page-title, .ls-block").waitForExist({ timeout: 20000 });
    // Use the user's quick switcher rather than a viewport-lazy journal link.
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 }); await input.setValue("Target");
    const result = await browser.$('.switcher-row[role="option"]');
    await result.waitForExist({ timeout: 10000 });
    await browser.waitUntil(() => browser.execute(() => {
      const row = document.querySelector('.switcher-row[role="option"][aria-selected="true"]');
      return row && !row.classList.contains("block-result") && row.textContent?.includes("Target");
    }), { timeout: 10000, timeoutMsg: "Target page was not the selected quick-switcher result" });
    await browser.keys(["Enter"]);
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".page-title")].some(e => e.textContent?.trim() === "Target")), { timeout: 15000, timeoutMsg: "Target route did not open" });
    console.log(`Native ${index}: Target opened`);
    const toggle = await browser.$('.linked-references button[aria-label="Filter linked references"]');
    await toggle.waitForExist({ timeout: 10000 }); await toggle.scrollIntoView(); await toggle.click();
    await browser.waitUntil(() => browser.execute(() => {
      const chips = [...document.querySelectorAll(".linked-references .ref-filter-chip")].map(e => e.textContent?.trim());
      return JSON.stringify(chips) === JSON.stringify(["Germany 4", "Zebra 1"]);
    }), { timeout: 10000, timeoutMsg: "Canonical alias chips or unique block counts are wrong" });
    console.log(`Native ${index}: Germany 4 / Zebra 1 chips visible`);
    await journey(browser);
  } catch (error) {
    if (browser) {
      await browser.saveScreenshot(path.join(TMP, `failure-${index}.png`)).catch(() => {});
      fs.writeFileSync(path.join(TMP, `failure-${index}.json`), JSON.stringify(await browser.execute(() => ({ text: document.body.innerText })).catch(() => null), null, 2));
    }
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
  }
}
async function expectCount(browser, count) {
  await browser.waitUntil(() => browser.execute(expected => document.querySelector(".linked-references .references-count")?.textContent === expected, String(count)), { timeout: 10000, timeoutMsg: `Reference count did not become ${count}` });
}

await withApp(0, async browser => {
  await expectCount(browser, 4); // alias-named legacy include covers all aliases
  await browser.$(".linked-references .ref-filter-chip.f-in").click();
  await expectCount(browser, 2); // Zebra plus the journal's navigation link
  await browser.waitUntil(() => fs.readFileSync(targetFile, "utf8").includes('filters:: {"Germany" false}'), { timeout: 10000, timeoutMsg: "Canonical filter did not reach the graph through guarded save" });
  // Restart proof must come from the graph property, not the device fallback.
  await browser.execute(() => localStorage.removeItem("logseq-claude.refFilters"));
  await browser.saveScreenshot(path.join(TMP, "excluded.png"));
});
await withApp(1, async browser => {
  await expectCount(browser, 2);
  if ((await browser.$(".linked-references .ref-filter-chip.f-out").getText()).trim() !== "Germany 4") throw new Error("Restart did not restore canonical exclusion");
  await browser.$(".linked-references .ref-filter-chip.f-out").click();
  await expectCount(browser, 6);
  await browser.waitUntil(() => fs.readFileSync(targetFile, "utf8").includes("filters:: {}"), { timeout: 10000, timeoutMsg: "Cleared filter did not persist" });
});
console.log(`PASS GH #433: canonical alias chips, legacy inclusion, alias exclusion, guarded disk save and restart (${TMP})`);
