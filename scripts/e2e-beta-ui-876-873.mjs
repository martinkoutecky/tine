// GH #876/#873: routed sheets paint Find and authored embed hosts preserve outline alignment.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.join(ROOT, "artifacts/fix-beta-ui-876-873");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-"));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const id = "87300000-0000-4000-8000-000000000001";
fs.writeFileSync(path.join(GRAPH, "pages/Source.md"), `- source root\n  id:: ${id}\n  - source child\n`);
const authoredPage = `- {{embed ((${id}))}}\n- {{embed ((${id}))}}\n  author:: agent\n- {{embed [[Source]]}}\n- {{embed [[Source]]}}\n  author:: agent\n- ordinary parent\n  author:: agent\n  - ordinary child\n`;
fs.writeFileSync(path.join(GRAPH, "pages/Author.md"), authoredPage);
fs.writeFileSync(path.join(GRAPH, "pages/Marked.md"), authoredPage.replaceAll("author::", "tine.author::"));
fs.writeFileSync(path.join(GRAPH, "pages/Task.md"), "- TODO query result\n");
fs.writeFileSync(path.join(GRAPH, "pages/Author siblings.md"), "- plain parent\n  - child\n- marked parent\n  tine.author:: agent\n  - child\n- {{query (task TODO)}}\n- {{query (task TODO)}}\n  tine.author:: agent\n");
for (const kind of ["grid", "table"]) {
  const rows = ["needle first", ...Array.from({ length: 220 }, (_, i) => `filler ${i}`), "needle second"];
  fs.writeFileSync(path.join(GRAPH, `pages/${kind}.md`), kind === "grid"
    ? `- Grid\n  tine.view:: grid\n${rows.map(text => `  -\n    - ${text}\n`).join("")}`
    : `- Table\n  tine.view:: table\n${rows.map(text => `  - ${text}\n`).join("")}`);
}
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const fixture = accountHomeFixture(RUN, RUN);
const env = { ...fixture.env, HOME: path.join(RUN, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"),
  XDG_CACHE_HOME: path.join(RUN, "cache"), XDG_STATE_HOME: path.join(RUN, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, stage = "startup";
const evidence = {};
// X11 capture avoids WebKitWebDriver's intermittent second-screenshot timeout.
const screenshot = name => execFileSync("import", ["-window", "root", path.join(RUN, name)], { timeout: 10000 });
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  if (process.env.E2E_CASE !== "sheet") {
  stage = "author alignment";
  await openPageByName(browser, "Author");
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .embed-block .bullet").length >= 4), { timeout: 15000 });
  assert.equal(await browser.execute(() => document.querySelectorAll(".main-content .ls-block.authored").length), 0,
    "bare author:: is an ordinary user property under D-20261102-PNS");
  await openPageByName(browser, "Marked");
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .embed-block .bullet").length >= 4), { timeout: 15000 });
  evidence.author = await browser.execute(() => {
    const embeds = [...document.querySelectorAll(".main-content .embed-block,.main-content .embed-page")];
    return embeds.map(embed => {
      const host = embed.closest(".ls-block");
      return { authored: host.classList.contains("authored"), x: embed.querySelector(".bullet").getBoundingClientRect().left,
        childX: embed.querySelector(".block-children .bullet")?.getBoundingClientRect().left };
    });
  });
  fs.writeFileSync(path.join(RUN, "author-metrics.json"), JSON.stringify(evidence.author, null, 2));
  screenshot("author.png");
  for (let i = 0; i < evidence.author.length; i += 2) {
    assert.equal(evidence.author[i].authored, false);
    assert.equal(evidence.author[i + 1].authored, true);
    assert.ok(Math.abs(evidence.author[i].x - evidence.author[i + 1].x) < 1, `authorship shifts embedded bullets: ${JSON.stringify(evidence.author)}`);
    assert.ok(Math.abs(evidence.author[i].childX - evidence.author[i + 1].childX) < 1, "authorship preserves child bullet alignment");
  }
  await openPageByName(browser, "Author siblings");
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .query-block").length === 2), { timeout: 15000 });
  evidence.siblings = await browser.execute(() => {
    const queries = [...document.querySelectorAll(".main-content .query-block")];
    const parents = [...document.querySelectorAll(".main-content .ls-block")].filter(block =>
      [...block.children].some(main => main.classList.contains("block-main") && /(?:plain|marked) parent/.test(main.textContent)));
    return { queryX: queries.map(query => query.getBoundingClientRect().left),
      childX: parents.map(parent => parent.querySelector(".block-children .bullet").getBoundingClientRect().left) };
  });
  assert.equal(evidence.siblings.childX.length, 2);
  assert.ok(Math.abs(evidence.siblings.childX[0] - evidence.siblings.childX[1]) < 1, "ordinary children stay aligned");
  assert.ok(Math.abs(evidence.siblings.queryX[0] - evidence.siblings.queryX[1]) < 1, "query contents stay aligned");
  screenshot("author-siblings.png");
  }
  if (process.env.E2E_CASE !== "author") {
  for (const kind of ["grid", "table"]) {
    stage = `${kind} Ctrl+F`;
    await openPageByName(browser, kind);
    await browser.$(`.sheet-${kind}`).waitForExist({ timeout: 15000 });
    await browser.keys(["Control", "f", "NULL"]);
    await browser.$(".inpage-find-input").setValue("needle");
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".inpage-find-count")?.textContent.includes("/ 2")), { timeout: 10000 });
    await browser.waitUntil(() => browser.execute(() => {
      const active = document.querySelector(".sheet-cell.inpage-find-active-block");
      const ranges = globalThis.CSS?.highlights?.get("tine-find-active");
      return !!active && (ranges?.size > 0 || !!document.querySelector(".inpage-find-overlay.active"));
    }), { timeout: 10000 });
    await browser.keys(["Enter"]);
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".sheet-cell.inpage-find-active-block")?.textContent.includes("second")), { timeout: 10000 });
    await browser.waitUntil(() => browser.execute(() => {
      const ranges = globalThis.CSS?.highlights?.get("tine-find-active");
      if (ranges) return [...ranges].some(range => range.startContainer.parentElement?.closest(".sheet-cell")?.textContent.includes("second"));
      const cell = document.querySelector(".sheet-cell.inpage-find-active-block")?.getBoundingClientRect();
      const paint = document.querySelector(".inpage-find-overlay.active")?.getBoundingClientRect();
      return cell && paint && paint.top >= cell.top && paint.bottom <= cell.bottom;
    }), { timeout: 10000, timeoutMsg: "second cell has no active painted occurrence" });
    assert.ok(await browser.execute(() => {
      const rect = document.querySelector(".sheet-cell.inpage-find-active-block").getBoundingClientRect();
      return rect.bottom > 0 && rect.top < innerHeight && rect.right > 0 && rect.left < innerWidth;
    }), "next matching cell is visible after crossing the render window");
    await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done())));
    screenshot(`${kind}.png`);
    await browser.keys(["Escape"]);
  }
  }
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ outcome: "PASS", app: APP, evidence }, null, 2));
  console.log(`PASS ${RUN}`);
} catch (error) {
  try { screenshot("failure.png"); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.txt"), `Base: 5a3b45a62 + lane diff\nJourney: ${stage}\nExpected: aligned author embeds and painted sheet Find\nObserved: ${error.stack}\nClassification: product if alignment/paint assertion, otherwise ambiguous harness/infrastructure\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
