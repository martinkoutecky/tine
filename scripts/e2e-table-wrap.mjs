// GH #802: reading tables expand into free pane space, then wrap in every host.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.TABLE_WRAP_OUT || path.join(root, "target/artifacts/og-tables-802");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
const table = cells => `| ${cells.map((_, i) => `Col ${i}`).join(" | ")} |\n| ${cells.map(() => "---").join(" | ")} |\n| ${cells.join(" | ")} |`;
const prose = "Cells grow into free pane space before their words wrap. ".repeat(10);
const tables = [table([prose, prose]), table(Array(6).fill("short cell words")),
  table([`https://example.test/${"segment/".repeat(24)}`, "link"]), table(["1", "2"])];
const outline = tables.map(text => `- ${text.replaceAll("\n", "\n  ")}\n`).join("");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(graph, "pages/Table reading.md"), outline);
fs.writeFileSync(path.join(graph, "pages/Embed reading.md"), "- {{embed [[Table reading]]}}\n- [[Table reading]]\n");
fs.writeFileSync(path.join(graph, "pages/Unbreakable.md"), `- ${table(["a".repeat(500), "end"]).replaceAll("\n", "\n  ")}\n`);
const date = new Date();
const day = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, `journals/${day}.md`), outline);
const snapshot = new Map(["pages", "journals"].flatMap(dir => fs.readdirSync(path.join(graph, dir)).map(name => {
  const file = path.join(graph, dir, name);
  return [file, fs.readFileSync(file, "utf8")];
})));
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4916);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const base = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const evidence = [];
let browser, stage = "startup";
const screenshot = name => execFileSync("import", ["-window", "root", path.join(out, `${name}.png`)], { timeout: 10000 });
async function checkReading(selector, name, count = 4) {
  stage = name;
  // Linked References may add further copies of an embed after the outline
  // loads; they must satisfy the same containment rule and are welcome here.
  await browser.waitUntil(() => browser.execute((selector, count) => document.querySelectorAll(`${selector} .md-table-wrap`).length >= count, selector, count), { timeout: 20000 });
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  // Copies arriving late (Linked References) are measured by TableWrap a frame
  // after they lay out; wait for the settled layout the reader sees, then assert.
  // A layout that never settles still fails below with its metrics.
  await browser.waitUntil(() => browser.execute(selector => [...document.querySelectorAll(`${selector} .md-table-wrap`)].every(wrap => {
    const pane = wrap.closest(".rs-item-body, .main-content");
    const r = wrap.getBoundingClientRect(), p = pane.getBoundingClientRect();
    return r.left >= p.left - 1 && r.right <= p.right + 1 && pane.scrollWidth - pane.clientWidth <= 1 && wrap.scrollWidth - wrap.clientWidth <= 1;
  }), selector), { timeout: 10000, interval: 200 }).catch(() => {});
  const metrics = await browser.execute(selector => [...document.querySelectorAll(`${selector} .md-table-wrap`)].map(wrap => {
    const pane = wrap.closest(".rs-item-body, .main-content");
    const r = wrap.getBoundingClientRect(), p = pane.getBoundingClientRect();
    return { width: r.width, scroll: wrap.scrollWidth - wrap.clientWidth,
      classes: wrap.className, inline: wrap.style.cssText,
      contained: r.left >= p.left - 1 && r.right <= p.right + 1,
      paneScroll: pane.scrollWidth - pane.clientWidth };
  }), selector);
  evidence.push({ name, metrics });
  fs.writeFileSync(path.join(out, `${name}-layout.json`), JSON.stringify(await browser.execute(selector =>
    [...document.querySelectorAll(`${selector} .md-table-wrap`)].map(wrap => {
      const table = wrap.querySelector("table"), style = getComputedStyle(table);
      return { classes: wrap.className, inline: wrap.style.cssText, width: wrap.clientWidth,
        table: { width: table.getBoundingClientRect().width, minWidth: style.minWidth, maxWidth: style.maxWidth },
        cells: [...table.querySelectorAll("th, td")].map(cell => ({ text: cell.textContent, classes: cell.className,
          width: cell.getBoundingClientRect().width, overflowWrap: getComputedStyle(cell).overflowWrap })),
        ancestors: [...(function* () { for (let node = wrap.parentElement; node; node = node.parentElement) yield node; })()]
          .map(node => ({ classes: node.className, width: node.clientWidth })) };
    }), selector), null, 2));
  screenshot(name);
  for (const metric of metrics) {
    assert.ok(metric.contained && metric.paneScroll <= 1, `${name}: table escapes pane: ${JSON.stringify(metric)}`);
    assert.ok(metric.scroll <= 1, `${name}: readable cell text needs horizontal scrolling: ${JSON.stringify(metric)}`);
  }
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  for (const mode of ["textarea", "cm6"]) {
    await browser.setWindowSize(1600, 1000);
    stage = `choose ${mode}`;
    await chooseEditor(browser, mode);
    await openPageByName(browser, "Table reading");
    await checkReading(".main-content", `${mode}-page`);
    // Dismissing the mobile drawer in the prior iteration also closes the
    // desktop left sidebar; make its Journals action available again.
    if (!await browser.execute(() => [...document.querySelectorAll(".nav-item")].some(el => el.textContent.trim() === "Journals"))) {
      await browser.keys(["t", "l"]);
      await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".nav-item")].some(el => el.textContent.trim() === "Journals")), { timeout: 5000 });
    }
    await openJournals(browser);
    await checkReading(".main-content", `${mode}-journal`);
    await openPageByName(browser, "Embed reading");
    await checkReading(".main-content .embed-page", `${mode}-embed`);
    await browser.execute(() => [...document.querySelectorAll(".main-content .page-ref")].find(el => el.textContent.includes("Table reading"))
      .dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true })));
    await checkReading(".right-sidebar", `${mode}-sidebar`);
    // Close the sidebar through its normal toggle before exercising the narrow pane.
    await browser.$('.right-sidebar-header .rs-close').click();
    await openPageByName(browser, "Table reading");
    await browser.setWindowSize(390, 900);
    // Desktop minimum width is 640px. Interface zoom reaches a genuine
    // phone-width CSS viewport through the same control a desktop user has.
    for (let i = 0; i < 10 && await browser.execute(() => innerWidth > 420); i++) {
      const before = await browser.execute(() => innerWidth);
      await browser.keys(["Control", "=", "NULL"]);
      await browser.waitUntil(() => browser.execute(before => innerWidth < before, before), { timeout: 5000 });
    }
    assert.ok(await browser.execute(() => innerWidth <= 420), "Native journey needs a phone-width CSS viewport");
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-mobile-drawer-scrim]")), { timeout: 5000 });
    await checkReading(".main-content", `${mode}-narrow`);
    evidence.push({ name: `${mode}-narrow-viewport`, width: await browser.execute(() => innerWidth) });
    await browser.keys(["Control", "0", "NULL"]);
    await browser.setWindowSize(1600, 1000);
    await openPageByName(browser, "Unbreakable");
    stage = `${mode}-local-scroll`;
    await browser.$(".main-content .md-table-wrap").waitForExist();
    const local = await browser.execute(() => {
      const wrap = document.querySelector(".main-content .md-table-wrap"), pane = wrap.closest(".main-content");
      wrap.scrollLeft = wrap.scrollWidth;
      return { moved: wrap.scrollLeft > 0, paneScroll: pane.scrollWidth - pane.clientWidth };
    });
    assert.ok(local.moved && local.paneScroll <= 1, `Unbreakable content must scroll inside its table: ${JSON.stringify(local)}`);
    evidence.push({ name: stage, local });
    screenshot(stage);
  }
  for (const [file, bytes] of snapshot) assert.equal(fs.readFileSync(file, "utf8"), bytes, `Reading changed ${file}`);
  fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify({ outcome: "PASS", base, app, evidence }, null, 2));
  console.log(`PASS ${out}`);
} catch (error) {
  try { screenshot("failure"); } catch {}
  fs.writeFileSync(path.join(out, "failure-evidence.json"), JSON.stringify(evidence, null, 2));
  fs.writeFileSync(path.join(out, "failure.txt"), `Base: ${base} + lane diff\nJourney: ${stage}\nExpected: wrapping tables contained in their pane; unbreakable content scrolls locally\nObserved: ${error.stack}\nClassification: ${error.code === "ERR_ASSERTION" ? "product" : "ambiguous harness/infrastructure"}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
